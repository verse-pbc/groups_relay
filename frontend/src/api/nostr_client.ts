import NDK, {
  NDKEvent,
  NDKPrivateKeySigner,
  NDKRelay,
  NDKPublishError,
  NDKUser,
} from "@nostr-dev-kit/ndk";
import { nip19 } from "nostr-tools";
import localforage from "localforage";

import type { Group } from "../types";
import { CashuWalletService, type ICashuWalletService, type Transaction } from "../services/CashuWalletService";

// NIP-29 event kinds
export enum GroupEventKind {
  JoinRequest = 9021,
  LeaveRequest = 9022,
  PutUser = 9000,
  RemoveUser = 9001,
  EditMetadata = 9002,
  DeleteEvent = 9005,
  CreateGroup = 9007,
  DeleteGroup = 9008,
  CreateInvite = 9009,
  GroupMetadata = 39000,
  GroupAdmins = 39001,
  GroupMembers = 39002,
}

export interface NostrClientConfig {
  relayUrl: string;
}

// Re-export Transaction type from wallet service
export { type Transaction } from "../services/CashuWalletService";

export class NostrGroupError extends Error {
  readonly rawMessage: string;

  constructor(message: string, context?: string) {
    super(context ? `${context}: ${message}` : message);
    this.name = "NostrGroupError";
    this.rawMessage = message;
  }

  get displayMessage(): string {
    return this.rawMessage;
  }
}

export class NostrClient {
  private groupsNdk: NDK;
  private globalNdk: NDK | null = null;
  readonly config: NostrClientConfig;
  private profileCache: Map<string, any> = new Map();
  private walletService: ICashuWalletService | null = null;
  private storageInitialized = false;
  private walletInitCallbacks: Set<() => void> = new Set();

  constructor(key: string, config?: Partial<NostrClientConfig>) {
    try {
      // Get WebSocket URL from environment variable or use current host
      const getWebSocketUrl = () => {
        // Check if we have an environment variable for the WebSocket URL
        if (
          typeof import.meta !== "undefined" &&
          import.meta.env &&
          import.meta.env.VITE_WEBSOCKET_URL
        ) {
          return import.meta.env.VITE_WEBSOCKET_URL;
        }

        // Otherwise, use the current host
        return `${window.location.protocol === "https:" ? "wss:" : "ws:"}//${window.location.host}`;
      };

      const defaultRelayUrl = getWebSocketUrl();

      this.config = {
        relayUrl: defaultRelayUrl,
        ...config,
      };

      // Validate the key format before creating the signer
      if (!key || typeof key !== "string") {
        throw new Error("Private key is required and must be a string");
      }

      // Try to create the signer with better error handling
      let signer;
      try {
        signer = new NDKPrivateKeySigner(key);
      } catch (signerError) {
        throw new Error(
          "Invalid private key provided. Please check the format and try again."
        );
      }

      // Groups NDK - only for group relay operations
      this.groupsNdk = new NDK({
        explicitRelayUrls: [
          this.config.relayUrl     // Group relay (current server) - connect to this first
        ],
        enableOutboxModel: false,        // Groups don't need outbox model
        autoConnectUserRelays: false,    // Groups operations are local only
        signer,
      });

      this.groupsNdk.pool.on("relay:connect", (relay: NDKRelay) => {
        console.log(`NDK relay connected: ${relay.url}, status: ${relay.status}`);
        // Use a custom auth policy that's more flexible with URL matching
        relay.authPolicy = async (relay: NDKRelay, challenge: string) => {
          try {
            // Get the signer from the NDK instance
            const signer = this.groupsNdk.signer;
            if (!signer) {
              throw new Error("No signer available");
            }

            // Create an auth event
            const authEvent = new NDKEvent(this.groupsNdk);
            authEvent.kind = 22242;
            
            // Remove trailing slash from relay URL to match server expectations
            const cleanRelayUrl = relay.url.replace(/\/$/, '');
            
            authEvent.tags = [
              ["relay", cleanRelayUrl],
              ["challenge", challenge]
            ];
            authEvent.created_at = Math.floor(Date.now() / 1000);

            // Sign the event
            await authEvent.sign(signer);

            // Return the signed event
            return authEvent;
          } catch (error) {
            console.error("Auth policy error:", error);
            throw error;
          }
        };
      });

      // Add error tracking for groups NDK instance
      this.groupsNdk.pool.on('relay:disconnect', (relay: NDKRelay) => {
        console.log(`NDK relay disconnected: ${relay.url}, status: ${relay.status}`);
        // Normal disconnections should trigger reconnection, not be counted as failures
        this.markRelayAsDead(relay.url, false);
      });
      
      // Track flapping relays (frequently connecting/disconnecting)
      this.groupsNdk.pool.on('flapping', (relay: NDKRelay) => {
        this.markRelayAsDead(relay.url, true, new Error('Relay is flapping'));
      });

      this.groupsNdk.pool.on('relay:auth', (relay: NDKRelay, _challenge: string) => {
        // Auth challenge received - set up auth failure detection
        const authTimeout = setTimeout(() => {
          this.markRelayAsDead(relay.url, true, new Error('Auth timeout'));
        }, 10000); // 10 second auth timeout
        
        // Clear timeout if auth succeeds
        relay.once('authed', () => clearTimeout(authTimeout));
        relay.once('disconnect', () => clearTimeout(authTimeout));
      });

      // Load temporarily dead relays from localStorage
      this.loadTemporarilyDeadRelays();

      // Initialize LocalForage
      this.initializeStorage();
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to initialize NostrClient"
      );
    }
  }

  // Initialize LocalForage storage
  private async initializeStorage(): Promise<void> {
    try {
      // Configure LocalForage
      localforage.config({
        name: 'cashu-wallet',
        storeName: 'wallet_data',
        description: 'Cashu wallet data storage'
      });

      // Load stored data
      const storedTransactions = await localforage.getItem<Transaction[]>('transactions');

      if (storedTransactions) {
        // Filter out invalid transactions (0 amount)
        const validTransactions = storedTransactions.filter(tx => tx.amount > 0);
        if (validTransactions.length !== storedTransactions.length) {
          // Update storage with valid transactions only
          await localforage.setItem('transactions', validTransactions);
        }
      } else {
      }

      this.storageInitialized = true;
    } catch (error) {
      // Storage initialization failed, but continue
    }
  }

  get ndkInstance(): NDK {
    return this.groupsNdk;
  }



  // Get transaction history
  getTransactionHistory(): Transaction[] {
    if (!this.walletService) return [];
    return this.walletService.getTransactionHistory();
  }

  // Clear expired wallet keys (older than 7 days)
  async clearExpiredWalletKeys(): Promise<void> {
    try {
      const keys = await localforage.keys();
      const walletKeyPrefix = 'wallet_keys_';
      const expiryTime = 7 * 24 * 60 * 60 * 1000; // 7 days
      
      for (const key of keys) {
        if (key.startsWith(walletKeyPrefix)) {
          const data = await localforage.getItem<any>(key);
          if (data && data.timestamp && Date.now() - data.timestamp > expiryTime) {
            await localforage.removeItem(key);
          }
        }
      }
    } catch (error) {
      // Failed to clear expired wallet keys, continue anyway
    }
  }


  // Ensure globalNdk is connected
  private async ensureGlobalNdk(): Promise<NDK> {
    // If globalNdk exists and is connected, return it
    if (this.globalNdk) {
      // Check if at least one relay is connected
      const relays = Array.from(this.globalNdk.pool.relays.values());
      const hasConnectedRelay = relays.some(relay => relay.status === 1);
      if (hasConnectedRelay) {
        return this.globalNdk;
      }
      
      // Wait a bit for connection if no relays are connected yet
      await new Promise(resolve => setTimeout(resolve, 1000));
      
      // Check again
      const hasConnectedRelayAfterWait = Array.from(this.globalNdk.pool.relays.values()).some(relay => relay.status === 1);
      if (hasConnectedRelayAfterWait) {
        return this.globalNdk;
      }
    }
    
    // Fallback to groupsNdk
    return this.groupsNdk;
  }
  
  // Ensure a specific relay is authenticated
  // private async ensureRelayAuthenticated(ndk: NDK, relayUrl: string): Promise<void> {
  //   const relay = Array.from(ndk.pool.relays.values()).find(r => 
  //     r.url.includes(relayUrl) || r.url.includes('localhost') || r.url.includes('example.local')
  //   );
    
  //   if (!relay) {
  //     console.warn(`Relay ${relayUrl} not found in pool`);
  //     return;
  //   }
    
  //   // Check if already ready (status 5 seems to indicate authenticated)
  //   if (relay.status === 5) {
  //     console.log(`Relay ${relayUrl} already ready/authenticated`);
  //     return;
  //   }
    
  //   // Wait for authentication
  //   console.log(`Waiting for relay ${relayUrl} to authenticate...`);
  //   await new Promise<void>((resolve) => {
  //     const timeout = setTimeout(() => {
  //       console.warn(`Authentication timeout for ${relayUrl}`);
  //       resolve();
  //     }, 3000);
      
  //     relay.once('authed', () => {
  //       console.log(`Relay ${relayUrl} authenticated`);
  //       clearTimeout(timeout);
  //       resolve();
  //     });
      
  //     // If already ready, resolve immediately
  //     if (relay.status === 5) {
  //       clearTimeout(timeout);
  //       resolve();
  //     }
  //   });
  // }

  // Wallet methods
  async initializeWallet(mints?: string[]): Promise<void> {
    try {
      // Ensure storage is initialized
      if (!this.storageInitialized) {
        await this.initializeStorage();
      }
      
      // Initialize wallet service if not already done
      if (!this.walletService) {
        // Ensure globalNdk is connected before creating wallet service
        const ndkForWallet = await this.ensureGlobalNdk();
        this.walletService = new CashuWalletService(ndkForWallet);
      } else {
        // Clear kind:10019 cache when re-initializing wallet (e.g., user switch)
        this.walletService.clearUser10019Cache();
      }
      
      await this.walletService.initializeWallet(mints);
      
      // Notify any listeners that wallet is now initialized
      this.walletInitCallbacks.forEach(callback => {
        try {
          callback();
        } catch (err) {
          // Error in wallet init callback, continue
        }
      });
      this.walletInitCallbacks.clear();
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to initialize wallet"
      );
    }
  }

  // Fetch existing NIP-60 wallet from user's relays
  async fetchNIP60Wallet(): Promise<any> {
    try {
      const user = await this.groupsNdk.signer?.user();
      if (!user) return null;

      // Use globalNdk which has outbox model enabled
      // It will automatically discover and use the user's preferred relays

      // Fetch NIP-60 wallet events (kinds 17375, 7375, 7376)
      const walletEventKinds = [17375, 7375, 7376];
      const filter = {
        kinds: walletEventKinds,
        authors: [user.pubkey],
        limit: 100
      };

      // Fetch from user's relays using globalNdk with outbox model
      const events = await (this.globalNdk || this.groupsNdk).fetchEvents(filter);
      
      if (events.size === 0) {
        return null;
      }
      

      // Create wallet instance with user's NDK (has user relays)
      // This is important for wallet operations to work properly
      const wallet = null; // Placeholder - actual wallet handled by CashuWalletService
      
      // For logging purposes, count what we found
      const detectedMints = new Set<string>();
      let tokenCount = 0;
      
      // Process wallet events for logging
      for (const event of Array.from(events)) {
        try {
          // Handle different NIP-60 event types
          if (event.kind === 17375) {
            // Wallet metadata - extract mints from tags
            const mintTags = event.tags.filter(tag => tag[0] === "mint" && tag[1]);
            if (mintTags.length > 0) {
              const eventMints = mintTags.map(tag => tag[1]);
              eventMints.forEach((mint: string) => detectedMints.add(mint));
            }
          } else if (event.kind === 7375) {
            // Cashu tokens
            tokenCount++;
            
            // Extract mint from tags if available
            const mintTag = event.tags.find(tag => tag[0] === 'mint');
            if (mintTag && mintTag[1]) {
              detectedMints.add(mintTag[1]);
            } else {
              // Check for 'u' tag as alternative (some implementations use 'u' for mint URL)
              const uTag = event.tags.find(tag => tag[0] === 'u');
              if (uTag && uTag[1]) {
                detectedMints.add(uTag[1]);
              } else {
                // Decrypt the NIP-44 encrypted content using NDK's built-in method
                try {
                  await event.decrypt();
                  
                  // Parse the decrypted content
                  try {
                    const tokenData = JSON.parse(event.content);
                    
                    // Check for mint in various formats
                    if (tokenData.mint) {
                      detectedMints.add(tokenData.mint);
                    } else if (tokenData.mints && Array.isArray(tokenData.mints) && tokenData.mints.length > 0) {
                      tokenData.mints.forEach((mint: string) => detectedMints.add(mint));
                    } else if (tokenData.token && typeof tokenData.token === 'string') {
                      // Try to decode the actual Cashu token
                      try {
                        const cashuToken = JSON.parse(atob(tokenData.token));
                        if (cashuToken.mint) {
                          detectedMints.add(cashuToken.mint);
                        }
                      } catch (e) {
                      }
                    }
                    // Proof amounts are available here if needed for future use
                    // tokenData.proofs contains the proof data
                  } catch (parseErr) {
                    // Could not parse decrypted content
                  }
                } catch (decryptErr) {
                  // Could not decrypt token content
                }
              }
            }
          }
        } catch (err) {
          // Error processing wallet event
        }
      }


      // Always return the wallet if we have any events
      // The wallet needs to process the events itself
      if (events.size > 0) {
        // If we have tokens without mint info, add common mints
        if (tokenCount > detectedMints.size) {
          // Add common mints that might have untagged tokens
          detectedMints.add('https://mint.minibits.cash');
          detectedMints.add('https://mint.minibits.cash/Bitcoin');
        }
        
        // Combine mints from metadata and detected from tokens
        const allMints = [...new Set([...Array.from(detectedMints)])];
        if (allMints.length > 0) {
        }
        
        return wallet;
      }

      return null; // No wallet data found
    } catch (error) {
      return null;
    }
  }

  // @deprecated Use walletService directly
  get walletInstance(): any {
    return this.walletService;
  }
  
  isWalletInitialized(): boolean {
    return this.walletService?.isInitialized() ?? false;
  }
  
  // Subscribe to wallet initialization
  onWalletInitialized(callback: () => void): () => void {
    // If already initialized, call immediately
    if (this.isWalletInitialized()) {
      callback();
      return () => {}; // No-op unsubscribe
    }
    
    // Otherwise add to callbacks
    this.walletInitCallbacks.add(callback);
    return () => {
      this.walletInitCallbacks.delete(callback);
    };
  }



  // Get all mints from wallet
  async getCashuMints(): Promise<string[]> {
    if (!this.walletService || !this.walletService.isInitialized()) {
      return [];
    }
    
    // Get mints from the wallet instance
    const wallet = (this.walletService as any).wallet;
    if (!wallet || !wallet.mints) {
      return [];
    }
    
    return wallet.mints;
  }

  // Get wallet proofs for nutzap functionality  
  getCashuProofs(): any[] {
    // NDKCashuWallet doesn't expose proofs directly
    // For nutzap functionality, we'll check if wallet has balance instead
    return [];
  }

  // Get wallet mints synchronously
  getWalletMints(): string[] {
    if (!this.walletService || !this.walletService.isInitialized()) {
      return [];
    }
    
    // Get mints from the wallet instance
    const wallet = (this.walletService as any).wallet;
    if (!wallet || !wallet.mints) {
      return [];
    }
    
    return wallet.mints;
  }

  // Check if wallet has any balance for nutzap functionality
  hasWalletBalance(): boolean {
    if (!this.walletService) {
      return false;
    }
    
    // Get current authorized balance instead of potentially stale cache
    // This is synchronous access to the wallet's current mint balances
    const wallet = (this.walletService as any).wallet;
    if (!wallet) return false;
    
    const mintBalances = wallet.mintBalances || {};
    const authorizedMints = wallet.mints || [];
    
    let authorizedBalance = 0;
    for (const [mint, balance] of Object.entries(mintBalances)) {
      if (authorizedMints.includes(mint) && typeof balance === 'number') {
        authorizedBalance += balance;
      }
    }
    
    return authorizedBalance > 0;
  }

  // Add a mint to the wallet and persist to NIP-60
  async addMint(mintUrl: string): Promise<void> {
    if (!this.walletService) throw new Error('Wallet not initialized');
    
    // Add mint to wallet
    await this.walletService.addMint(mintUrl);
    
    // Update the kind:10019 event to include the new mint
    await this.walletService.publishNutzapConfig();
  }

  // Remove a mint from the wallet and persist to NIP-60
  async removeMint(mintUrl: string): Promise<void> {
    if (!this.walletService) throw new Error('Wallet not initialized');
    
    await this.walletService.removeMint(mintUrl);
  }
  

  // Get balance for a specific mint
  async getCashuBalance(mintUrl?: string): Promise<number> {
    if (!this.walletService) {
      return 0;
    }
    
    if (mintUrl) {
      return 0;
    }
    
    const balance = await this.walletService.getBalance();
    return balance;
  }

  // Get balance available for sending to a specific recipient
  async getCashuBalanceForRecipient(recipientPubkey: string): Promise<number> {
    if (!this.walletService) {
      return 0;
    }
    
    const balance = await this.walletService.getBalanceForRecipient(recipientPubkey);
    return balance;
  }

  async getCashuMintBalances(): Promise<Record<string, number>> {
    if (!this.walletService) {
      return {};
    }
    
    const balances = await this.walletService.getMintBalances();
    return balances;
  }

  async getAllCashuMintBalances(): Promise<{ authorized: Record<string, number>, unauthorized: Record<string, number> }> {
    if (!this.walletService) return { authorized: {}, unauthorized: {} };
    
    return this.walletService.getAllMintBalances();
  }



  // Get cached balance immediately without async call
  getCachedBalance(): number {
    // Create wallet service if needed (to access cached balance)
    if (!this.walletService) {
      this.walletService = new CashuWalletService(this.globalNdk || this.groupsNdk);
    }
    return this.walletService.getCachedBalance();
  }

  // Get cached balance for a specific user
  getCachedBalanceForUser(userPubkey: string): number {
    // Create wallet service if needed
    if (!this.walletService) {
      this.walletService = new CashuWalletService(this.globalNdk || this.groupsNdk);
    }
    return this.walletService.loadCachedBalanceForUser(userPubkey);
  }

  // Subscribe to balance updates
  onBalanceUpdate(callback: (balance: number) => void): () => void {
    // Create wallet service if needed (to enable subscriptions)
    if (!this.walletService) {
      this.walletService = new CashuWalletService(this.globalNdk || this.groupsNdk);
    }
    return this.walletService.onBalanceUpdate(callback);
  }

  // Optimistically update balance (e.g., after sending nutzap)
  notifyBalanceUpdate(newBalance: number): void {
    if (!this.walletService) return;
    this.walletService.updateBalanceOptimistically(newBalance);
  }

  // Mint tokens using wallet service
  async mintTokens(mintUrl: string, amount: number): Promise<{ proofs: any[], invoice: string, quote: any }> {
    if (!this.walletService) {
      throw new Error("Wallet not initialized");
    }
    
    const result = await this.walletService.mintTokens(mintUrl, amount);
    return { proofs: [], ...result };
  }
  async checkAndClaimTokens(mintUrl: string, quote: any): Promise<{ proofs: any[], claimed: boolean }> {
    if (!this.walletService) {
      throw new Error("Wallet not initialized");
    }
    
    return this.walletService.checkAndClaimTokens(mintUrl, quote);
  }

  async meltToLightning(invoice: string, selectedMint?: string): Promise<{ paid: boolean; preimage?: string; fee?: number; error?: string }> {
    if (!this.walletService) {
      throw new Error("Wallet not initialized");
    }
    
    return this.walletService.meltToLightning(invoice, selectedMint);
  }


  async receiveTokens(token: string): Promise<{ proofs: any[], amount: number }> {
    if (!this.walletService) {
      throw new Error("Wallet not initialized");
    }
    
    const result = await this.walletService.receiveTokens(token);
    return { proofs: [], amount: result.amount };
  }


  // Send nutzap to an event
  async sendNutzapToEvent(eventId: string, amount: number, mint?: string, nutzapRelays?: string[] | null, groupId?: string): Promise<void> {
    if (!this.walletService) {
      throw new Error('Wallet not initialized')
    }

    // First, we need to find the event author's pubkey
    const filter = {
      ids: [eventId],
      limit: 1
    };
    
    // Try to fetch event from both NDK instances (global and groups)
    // Group events might only exist on the local relay
    let event: any = null;
    
    // First try globalNdk
    try {
      const ndkToUse = await this.ensureGlobalNdk();
      const fetchPromise = ndkToUse.fetchEvents(filter);
      const timeoutPromise = new Promise((_, reject) => 
        setTimeout(() => reject(new Error("Timeout")), 2000)
      );
      
      const events = await Promise.race([fetchPromise, timeoutPromise]) as Set<any>;
      if (events.size > 0) {
        event = Array.from(events)[0];
      }
    } catch (err) {
      // Event not found on global relays, trying local relay...
    }
    
    // If not found, try groupsNdk (local relay) using subscription approach
    if (!event) {
      try {
        
        // Check current authentication status
        const localRelay = Array.from(this.groupsNdk.pool.relays.values()).find(r => 
          r.url.includes('localhost') || r.url.includes('example.local') || r.url.includes('8080')
        );
        
        if (localRelay) {
          // Check local relay status
        }
        
        // Use subscription approach which handles auth better
        const sub = this.groupsNdk.subscribe(filter, { 
          closeOnEose: true,
          groupable: false 
        });
        
        // Wait for event with timeout
        event = await new Promise((resolve) => {
          const timeout = setTimeout(() => {
            sub.stop();
            resolve(null);
          }, 5000); // Increased timeout to allow for auth
          
          sub.on('event', (e: any) => {
            clearTimeout(timeout);
            sub.stop();
            resolve(e);
          });
          
          sub.on('eose', () => {
            clearTimeout(timeout);
            sub.stop();
            resolve(null);
          });
        });
        
        // If still no event but relay got authenticated during subscription, try fetchEvents
        if (!event && localRelay && localRelay.status >= 5) {
          const events = await this.groupsNdk.fetchEvents(filter);
          if (events.size > 0) {
            event = Array.from(events)[0];
          }
        }
      } catch (err) {
        // Error fetching from local relay
      }
    }
    
    if (!event) {
      throw new Error('Event not found - make sure the event exists on accessible relays');
    }
    
    const authorPubkey = event.pubkey;
    
    // If nutzapRelays not provided, fetch the author's 10019 event to get their nutzap relays
    if (!nutzapRelays) {
      const event10019 = await this.walletService!.fetchUser10019(authorPubkey);
      nutzapRelays = event10019 ? this.walletService!.parseNutzapRelays(event10019) : null;
    }

    // Pass the event object to wallet service since it might not have access to local relay
    await this.walletService.sendNutzapToEvent(eventId, amount, mint, nutzapRelays, groupId, event);
  }


  // Send nutzap to a user
  async sendNutzap(pubkey: string, amount: number, mint?: string, groupId?: string): Promise<void> {
    if (!this.walletService) {
      throw new Error('Wallet not initialized')
    }

    // Fetch the user's 10019 event to get their nutzap relays
    const event10019 = await this.walletService!.fetchUser10019(pubkey);
    const nutzapRelays = event10019 ? this.walletService!.parseNutzapRelays(event10019) : null;

    await this.walletService.sendNutzap(pubkey, amount, mint, nutzapRelays, groupId);
  }

  // Public methods for fetching events and subscribing
  async fetchEvents(filter: any): Promise<any[]> {
    const events = await this.groupsNdk.fetchEvents(filter)
    return Array.from(events)
  }

  async subscribe(filter: any, options?: any): Promise<any> {
    return this.groupsNdk.subscribe(filter, options)
  }

  // Minimal list of relays that are persistently problematic (DNS failures, etc)
  // Most dead relay detection should be dynamic and temporary
  private persistentlyDeadRelays = new Set([
    'wss://relay.nostr.bg',  // DNS: no such host
    'wss://relay.current.fyi',  // DNS: no such host  
    'wss://relay.causes.com'  // DNS: no such host
  ]);

  // Dynamic dead relay list (persists across reloads with expiration)
  private temporarilyDeadRelays = new Set<string>();

  // Track failed connection attempts with timestamps
  private relayFailures = new Map<string, { count: number, lastFailed: number }>();

  // Load temporarily dead relays from localStorage with expiration
  private loadTemporarilyDeadRelays(): void {
    try {
      const stored = localStorage.getItem('temporarilyDeadRelays');
      if (stored) {
        const data = JSON.parse(stored);
        const now = Date.now();
        const expiredTime = 2 * 60 * 60 * 1000; // 2 hours
        
        // Only keep relays that haven't expired
        Object.entries(data).forEach(([relay, timestamp]) => {
          if (now - (timestamp as number) < expiredTime) {
            this.temporarilyDeadRelays.add(relay);
          }
        });
      }
    } catch (error) {
      // Failed to load temporarily dead relays
    }
  }

  // Save temporarily dead relays to localStorage
  private saveTemporarilyDeadRelays(): void {
    try {
      const now = Date.now();
      const data: Record<string, number> = {};
      this.temporarilyDeadRelays.forEach(relay => {
        data[relay] = now;
      });
      localStorage.setItem('temporarilyDeadRelays', JSON.stringify(data));
    } catch (error) {
      // Failed to save temporarily dead relays
    }
  }

  // Check if a disconnection is a normal server-side disconnection
  private isNormalDisconnection(error?: Error): boolean {
    if (!error?.message) return false;
    
    const normalDisconnectionPatterns = [
      /max connection time.*exceeded.*initiating graceful shutdown/i,
      /connection timeout/i,
      /server is shutting down/i,
      /graceful shutdown/i,
      /connection closed by server/i
    ];
    
    return normalDisconnectionPatterns.some(pattern => pattern.test(error.message));
  }

  // Attempt to reconnect to a relay immediately
  private async attemptReconnection(relayUrl: string): Promise<void> {
    try {
      // Find the relay in our groups NDK pool and attempt reconnection
      const relay = this.groupsNdk.pool.relays.get(relayUrl);
      
      if (relay && relay.status !== 1) { // Not connected
        await this.connectToRelay(relay);
      }
    } catch (error) {
      // Failed to reconnect
    }
  }

  // Connect to a specific relay with timeout
  private async connectToRelay(relay: NDKRelay): Promise<void> {
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        reject(new Error('Connection timeout'));
      }, 5000);
      
      const onConnect = () => {
        clearTimeout(timeout);
        relay.removeListener('connect', onConnect);
        relay.removeListener('disconnect', onDisconnect);
        resolve();
      };
      
      const onDisconnect = (error?: Error) => {
        clearTimeout(timeout);
        relay.removeListener('connect', onConnect);
        relay.removeListener('disconnect', onDisconnect);
        reject(error || new Error('Disconnected during connection attempt'));
      };
      
      relay.on('connect', onConnect);
      relay.on('disconnect', onDisconnect);
      
      // If already connected, resolve immediately
      if (relay.status === 1) {
        clearTimeout(timeout);
        resolve();
        return;
      }
      
      // Attempt connection
      relay.connect().catch(reject);
    });
  }

  // Add a relay to the temporary dead list if it fails repeatedly
  private markRelayAsDead(relayUrl: string, isActualFailure: boolean = true, error?: Error): void {
    // Handle normal disconnections differently
    if (!isActualFailure && this.isNormalDisconnection(error)) {
      // Attempt immediate reconnection for normal disconnections
      this.attemptReconnection(relayUrl).catch(err => {
        // If reconnection fails, treat as actual failure
        this.markRelayAsDead(relayUrl, true, err instanceof Error ? err : new Error(String(err)));
      });
      return;
    }
    
    // Only count actual failures toward dead relay threshold
    if (!isActualFailure) {
      this.attemptReconnection(relayUrl);
      return;
    }
    
    const now = Date.now();
    const existing = this.relayFailures.get(relayUrl) || { count: 0, lastFailed: 0 };
    
    // Reset count if it's been more than 1 hour since last failure
    if (now - existing.lastFailed > 3600000) {
      existing.count = 0;
    }
    
    existing.count++;
    existing.lastFailed = now;
    this.relayFailures.set(relayUrl, existing);
    
    // Mark as temporarily dead after 6 failures (persists for 2 hours)
    if (existing.count >= 6) {
      this.temporarilyDeadRelays.add(relayUrl);
      this.saveTemporarilyDeadRelays(); // Persist to localStorage
    }
  }

  // Filter out dead relays and limit count for general use
  private filterHealthyRelays(relays: string[]): string[] {
    return relays
      .filter(relay => 
        !this.persistentlyDeadRelays.has(relay) && 
        !this.temporarilyDeadRelays.has(relay)
      )
      .slice(0, 3); // Conservative limit to prevent WebSocket exhaustion
  }


  // Get user's preferred relays from NIP-65
  async getUserRelays(pubkey: string): Promise<string[]> {
    try {
      // Fetch NIP-65 relay list (kind 10002)
      const filter = {
        kinds: [10002],
        authors: [pubkey],
        limit: 1
      };
      
      const events = await this.groupsNdk.fetchEvents(filter);
      if (events.size === 0) {
        const fallbackRelays = ["wss://relay.damus.io", "wss://relay.nos.social", "wss://relay.primal.net"];
        return this.filterHealthyRelays(fallbackRelays);
      }
      
      const relayListEvent = Array.from(events)[0];
      const relays: string[] = [];
      
      // Parse relay tags
      relayListEvent.tags.forEach(tag => {
        if (tag[0] === 'r' && tag[1]) {
          relays.push(tag[1]);
        }
      });
      
      if (relays.length === 0) {
        const fallbackRelays = ["wss://relay.damus.io", "wss://relay.nos.social"];
        return this.filterHealthyRelays(fallbackRelays);
      }
      
      return this.filterHealthyRelays(relays);
    } catch (error) {
      const fallbackRelays = ["wss://relay.damus.io", "wss://relay.nos.social"];
      return this.filterHealthyRelays(fallbackRelays);
    }
  }


  // Note: Group write relay management removed - NDK outbox model handles this automatically


  /**
   * Get the wallet service instance for Cashu operations
   */
  getWalletService(): ICashuWalletService | null {
    return this.walletService;
  }

  async connect() {
    try {
      console.log('NostrClient: Connecting to relay...');
      await this.groupsNdk.connect();
      console.log('NostrClient: Connected successfully');
      
      // Don't create wallet service here - it will be created when needed with proper NDK instance

      // Wait specifically for the main relay to be ready
      const mainRelayUrl = this.config.relayUrl;
      
      // Normalize URL by removing trailing slash
      const normalizeUrl = (url: string) => url.replace(/\/$/, '');
      const normalizedMainUrl = normalizeUrl(mainRelayUrl);
      
      // Wait a bit for relay to be added to pool
      let attempts = 0;
      let mainRelay: NDKRelay | undefined;
      
      while (!mainRelay && attempts < 20) {
        mainRelay = Array.from(this.groupsNdk.pool.relays.values()).find(
          relay => normalizeUrl(relay.url) === normalizedMainUrl
        );
        
        if (!mainRelay) {
          // Wait 100ms and try again
          await new Promise(resolve => setTimeout(resolve, 100));
          attempts++;
        }
      }

      if (!mainRelay) {
        // List all relays in pool for debugging
        throw new Error(`Main relay ${mainRelayUrl} not found in pool after ${attempts} attempts`);
      }

      // Wait for the main relay to be ready
      await new Promise<void>((resolve, reject) => {
        // Check if already ready (status 5 = READY)
        if (mainRelay.status === 5) {
          resolve();
          return;
        }

        const handleStatus = () => {
          if (mainRelay.status === 5) {
            cleanup();
            resolve();
          }
        };

        const handleError = (err: Error) => {
          cleanup();
          reject(err);
        };

        // Setup event listeners
        mainRelay.on("authed", () => {
          cleanup();
          resolve();
        });
        mainRelay.on("disconnect", () =>
          handleError(new Error("Main relay disconnected"))
        );
        mainRelay.on("auth:failed", (err) =>
          handleError(new Error(`Main relay auth failed: ${err.message}`))
        );

        const interval = setInterval(handleStatus, 100);

        const cleanup = () => {
          clearInterval(interval);
          mainRelay.removeAllListeners("authed");
          mainRelay.removeAllListeners("disconnect");
          mainRelay.removeAllListeners("auth:failed");
        };

        // Increase timeout for main relay
        setTimeout(() => {
          cleanup();
          reject(
            new Error("Connection timeout waiting for main relay authentication")
          );
        }, 10000); // 10 seconds for main relay
      });

      // All relay statuses are now available
      
      // Now create a separate NDK instance for all non-group operations with public relays
      try {
        this.globalNdk = new NDK({
          explicitRelayUrls: [
            // Don't include local relay here - it's already handled by groupsNdk
            // and having it in both causes auth conflicts with subdomain validation
            "wss://relay.damus.io",   // Popular relay
            "wss://relay.nos.social",  // NOS relay
            "wss://purplepag.es",      // Profile relay
            "wss://relay.primal.net",  // Primal relay
            "wss://nos.lol"            // Popular relay
          ],
          enableOutboxModel: true,
          autoConnectUserRelays: true,
          signer: this.groupsNdk.signer
        });
        
        // Connect global NDK in the background
        this.globalNdk.connect().catch(() => {
          // Global NDK connection failed, some features may not work
        });
      } catch {
        // Failed to create global NDK
      }
    } catch (error) {
      throw new NostrGroupError(`Failed to connect: ${error}`);
    }
  }

  async disconnect() {
    try {
      // Close all relay connections from the groups NDK instance
      const relays = Array.from(this.groupsNdk.pool.relays.values());
      await Promise.all(relays.map((relay) => relay.disconnect()));

      // Clear any subscriptions
      this.groupsNdk.pool.removeAllListeners();
      
      // Disconnect global NDK if it exists
      if (this.globalNdk) {
        const globalRelays = Array.from(this.globalNdk.pool.relays.values());
        await Promise.all(globalRelays.map((relay) => relay.disconnect()));
        this.globalNdk.pool.removeAllListeners();
        this.globalNdk = null;
      }

      // Clear profile cache
      this.profileCache.clear();

      // Dispose wallet service if it exists
      if (this.walletService) {
        this.walletService.dispose();
        this.walletService = null;
      }

      // Clear localStorage entries (except nostr_key which is handled by main.tsx)
      const keysToRemove = [
        'temporarilyDeadRelays',
        'cashu_transactions'
      ];
      
      // Also clear any user-specific balance keys
      for (let i = 0; i < localStorage.length; i++) {
        const key = localStorage.key(i);
        if (key && key.startsWith('cashu_balance_')) {
          keysToRemove.push(key);
        }
      }
      
      keysToRemove.forEach(key => localStorage.removeItem(key));

      // Clear localforage data
      try {
        await localforage.clear();
      } catch (error) {
        // Failed to clear localforage
      }

      // Clear relay failure tracking
      this.relayFailures.clear();
      this.temporarilyDeadRelays.clear();

      // Reset storage initialization flag
      this.storageInitialized = false;

    } catch (error) {
      throw new NostrGroupError(`Failed to disconnect: ${error}`);
    }
  }

  private async publishEvent(
    kind: GroupEventKind,
    tags: string[][],
    content: string = ""
  ) {
    try {
      // Ensure we have a relay in READY state (status 5)
      const readyRelays = Array.from(this.groupsNdk.pool.relays.values()).filter(
        (r) => r.status === 5
      );

      if (readyRelays.length === 0) {
        throw new NostrGroupError(
          "Please ensure you are authenticated.",
          "No ready relays available"
        );
      }

      const ndkEvent = new NDKEvent(this.groupsNdk);
      ndkEvent.kind = kind;
      ndkEvent.tags = tags;
      ndkEvent.content = content;
      await ndkEvent.sign();

      await ndkEvent.publish();

      return ndkEvent;
    } catch (error) {
      // If it's a NDKPublishError, we can get specific relay errors
      if (error instanceof NDKPublishError) {
        // Get the first relay error (there's usually only one relay in our case)
        for (const [relay, err] of error.errors) {
          // The backend now sends proper error messages in OK responses
          // Format is usually "error: <message>" from the error handling middleware
          const errorMessage = err.message;
          
          // Remove "error: " prefix if present
          const cleanMessage = errorMessage.replace(/^error:\s*/i, '');
          
          throw new NostrGroupError(cleanMessage, relay.url);
        }
      }

      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to publish event"
      );
    }
  }

  async sendJoinRequest(groupId: string, inviteCode?: string) {
    const tags = [["h", groupId]];
    if (inviteCode) {
      tags.push(["code", inviteCode]);
    }
    return this.publishEvent(GroupEventKind.JoinRequest, tags);
  }

  async acceptJoinRequest(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, "member"],
    ]);
  }

  async createGroup(group: Group) {
    // First create the group
    await this.publishEvent(GroupEventKind.CreateGroup, [["h", group.id]]);

    // Then set its metadata
    const metadataTags = [["h", group.id]];
    if (group.name) metadataTags.push(["name", group.name]);
    if (group.about) metadataTags.push(["about", group.about]);
    if (group.picture) metadataTags.push(["picture", group.picture]);
    metadataTags.push([group.private ? "private" : "public"]);
    metadataTags.push([group.closed ? "closed" : "open"]);
    metadataTags.push([group.broadcast ? "broadcast" : "nonbroadcast"]);

    await this.publishEvent(GroupEventKind.EditMetadata, metadataTags);
    return group;
  }

  async updateGroupName(groupId: string, newName: string) {
    return this.publishEvent(GroupEventKind.EditMetadata, [
      ["h", groupId],
      ["name", newName],
    ]);
  }

  async updateGroupMetadata(group: Group) {
    const tags = [["h", group.id]];
    if (group.name) tags.push(["name", group.name]);
    if (group.picture) tags.push(["picture", group.picture]);
    if (group.about) tags.push(["about", group.about]);
    tags.push([group.private ? "private" : "public"]);
    tags.push([group.closed ? "closed" : "open"]);
    tags.push([group.broadcast ? "broadcast" : "nonbroadcast"]);

    return this.publishEvent(GroupEventKind.EditMetadata, tags);
  }

  async leaveGroup(groupId: string) {
    return this.publishEvent(GroupEventKind.LeaveRequest, [["h", groupId]]);
  }

  async addModerator(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, "moderator"],
    ]);
  }

  async removeModerator(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.RemoveUser, [
      ["h", groupId],
      ["p", pubkey],
    ]);
  }

  async removeMember(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.RemoveUser, [
      ["h", groupId],
      ["p", pubkey],
    ]);
  }

  async addMember(groupId: string, pubkey: string) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, "member"],
    ]);
  }

  async toggleAdminRole(groupId: string, pubkey: string, isAdmin: boolean) {
    return this.publishEvent(GroupEventKind.PutUser, [
      ["h", groupId],
      ["p", pubkey, isAdmin ? "Admin" : "Member"],
    ]);
  }

  async createInvite(groupId: string, code: string) {
    return this.publishEvent(GroupEventKind.CreateInvite, [
      ["h", groupId],
      ["code", code],
      ["roles", "member"],
    ]);
  }

  async deleteEvent(groupId: string, eventId: string) {
    return this.publishEvent(GroupEventKind.DeleteEvent, [
      ["h", groupId],
      ["e", eventId],
    ]);
  }

  async deleteGroup(groupId: string) {
    return this.publishEvent(GroupEventKind.DeleteGroup, [["h", groupId]]);
  }

  async fetchProfile(pubkey: string) {
    try {
      // Check cache first
      if (this.profileCache.has(pubkey)) {
        return this.profileCache.get(pubkey);
      }

      const user = new NDKUser({ pubkey });
      // Use profile NDK if available, otherwise fall back to main NDK
      user.ndk = this.globalNdk || this.groupsNdk;
      await user.fetchProfile();

      // Cache the profile
      if (user.profile) {
        this.profileCache.set(pubkey, user.profile);
      }

      return user.profile;
    } catch (error) {
      return null;
    }
  }

  // Convert a hex pubkey to npub
  pubkeyToNpub(pubkey: string): string {
    try {
      return nip19.npubEncode(pubkey);
    } catch (error) {
      return pubkey;
    }
  }

  // Convert an npub to hex pubkey
  npubToPubkey(npub: string): string {
    try {
      const { type, data } = nip19.decode(npub);
      if (type !== "npub") {
        throw new Error("Not an npub");
      }
      return data as string;
    } catch (error) {
      throw new NostrGroupError("Invalid npub format");
    }
  }

  // Resolve a NIP-05 address to a pubkey
  async resolveNip05(nip05Address: string): Promise<string> {
    try {
      const [name, domain] = nip05Address.split("@");
      if (!name || !domain) {
        throw new Error("Invalid NIP-05 format");
      }

      const response = await fetch(
        `https://${domain}/.well-known/nostr.json?name=${name}`
      );
      if (!response.ok) {
        throw new Error("Failed to fetch NIP-05 data");
      }

      const data = await response.json();
      const pubkey = data?.names?.[name];
      if (!pubkey) {
        throw new Error("NIP-05 address not found");
      }

      return pubkey;
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : "Failed to resolve NIP-05"
      );
    }
  }

  async checkIsRelayAdmin(): Promise<boolean> {
    try {
      const user = await this.ndkInstance.signer?.user();
      if (!user?.pubkey) return false;

      const httpUrl = this.config.relayUrl
        .replace(/^wss?:\/\//, (match) =>
          match === "ws://" ? "http://" : "https://"
        )
        .replace(/\/$/, "");

      const response = await fetch(httpUrl, {
        method: "GET",
        mode: "cors",
        credentials: "omit",
        cache: "no-cache",
        headers: {
          Accept: "application/nostr+json",
          "Cache-Control": "no-cache",
          Pragma: "no-cache",
        },
      });

      const contentType = response.headers.get("content-type");
      if (
        response.ok &&
        (contentType?.includes("application/json") ||
          contentType?.includes("application/nostr+json"))
      ) {
        const relayInfo = await response.json();
        return relayInfo.pubkey === user.pubkey;
      }

      return false;
    } catch (error) {
      return false;
    }
  }
}

export function hashGroup(group: Group): string {
  const { id, name, invites, joinRequests: join_requests, content } = group;
  return JSON.stringify({ id, name, invites, join_requests, content });
}
