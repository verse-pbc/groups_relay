import NDK, {
  NDKEvent,
  NDKPrivateKeySigner,
  NDKRelay,
  NDKRelayAuthPolicies,
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
  private ndk: NDK;
  private profileNdk: NDK;
  private groupWriteNdk: NDK | null = null;
  private groupWriteRelays: Set<string> = new Set();
  readonly config: NostrClientConfig;
  private profileCache: Map<string, any> = new Map();
  private user10019Cache: Map<string, any> = new Map();
  private user10019FetchPromises: Map<string, Promise<any>> = new Map(); // Prevent duplicate fetches
  private walletService: ICashuWalletService | null = null;
  private storageInitialized = false;

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
        console.error("Failed to create NDKPrivateKeySigner:", signerError);
        throw new Error(
          "Invalid private key provided. Please check the format and try again."
        );
      }

      // Main NDK instance for group operations
      this.ndk = new NDK({
        explicitRelayUrls: [this.config.relayUrl],
        signer,
      });

      // Separate NDK instance for profile fetching
      // Include the current relay in addition to public relays
      this.profileNdk = new NDK({
        explicitRelayUrls: [
          this.config.relayUrl,  // Include current relay
          "wss://relay.nos.social", 
          "wss://purplepag.es"
        ],
        signer,  // Add signer for authentication
      });

      this.ndk.pool.on("relay:connect", (relay: NDKRelay) => {
        relay.authPolicy = NDKRelayAuthPolicies.signIn({ ndk: this.ndk });
      });

      this.profileNdk.pool.on("relay:connect", (relay: NDKRelay) => {
        relay.authPolicy = NDKRelayAuthPolicies.signIn({ ndk: this.profileNdk });
      });

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
        // Transactions are now managed by CashuWalletService
      } else {
      }

      this.storageInitialized = true;
    } catch (error) {
      console.error('Failed to initialize storage:', error);
    }
  }

  get ndkInstance(): NDK {
    return this.ndk;
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
      console.error('Failed to clear expired wallet keys:', error);
    }
  }


  // Wallet methods
  async initializeWallet(mints?: string[]): Promise<void> {
    try {
      // Ensure storage is initialized
      if (!this.storageInitialized) {
        await this.initializeStorage();
      }
      
      // Initialize wallet service if not already done
      if (!this.walletService) {
        this.walletService = new CashuWalletService(this.ndk);
      }
      
      await this.walletService.initializeWallet(mints);
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
      const user = await this.ndk.signer?.user();
      if (!user) return null;

      // First get user's relay list to know where to look
      const userRelays = await this.getUserRelays(user.pubkey);
      
      // Create a temporary NDK instance with user's relays
      const userNdk = new NDK({
        explicitRelayUrls: userRelays,
        signer: this.ndk.signer
      });
      
      // Connect with timeout
      const connectPromise = userNdk.connect();
      const timeoutPromise = new Promise((_, reject) => 
        setTimeout(() => reject(new Error("Connection timeout")), 3000)
      );
      
      try {
        await Promise.race([connectPromise, timeoutPromise]);
      } catch (err) {
        // Continue anyway - some relays might be connected
      }

      // Fetch NIP-60 wallet events (kinds 17375, 7375, 7376)
      const walletEventKinds = [17375, 7375, 7376];
      const filter = {
        kinds: walletEventKinds,
        authors: [user.pubkey],
        limit: 100
      };

      // Fetch from user's relays
      const events = await userNdk.fetchEvents(filter);
      
      if (events.size === 0) {
        return null;
      }
      

      // Create wallet instance with user's NDK (has user relays)
      // This is important for wallet operations to work properly
      // Note: NDKCashuWallet is now handled by CashuWalletService
      const wallet = null; // Placeholder
      
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
              // Mints are now managed by CashuWalletService
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
                    // Log proof amounts to understand denominations
                    if (tokenData.proofs && Array.isArray(tokenData.proofs)) {
                      const amounts = tokenData.proofs.map((p: any) => p.amount);
                      console.log(`💰 Proof denominations: ${amounts.join(', ')} (total: ${amounts.reduce((a: number, b: number) => a + b, 0)})`);
                    }
                  } catch (parseErr) {
                    console.log(`⚠️ Could not parse decrypted content:`, parseErr);
                  }
                } catch (decryptErr) {
                  console.log(`⚠️ Could not decrypt token content:`, decryptErr);
                }
              }
            }
          }
        } catch (err) {
          console.warn(`⚠️ Error processing wallet event:`, err);
        }
      }


      // Balance caching is now handled by CashuWalletService

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
        
        // NDK instance is now managed by CashuWalletService
        
        return wallet;
      }

      return null; // No wallet data found
    } catch (error) {
      console.error("❌ Failed to fetch NIP-60 wallet:", error);
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
    return this.walletService !== null;
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
    if (!this.walletService) return 0;
    
    if (mintUrl) {
      return 0;
    }
    
    return this.walletService.getBalance();
  }

  async getCashuMintBalances(): Promise<Record<string, number>> {
    if (!this.walletService) return {};
    
    return this.walletService.getMintBalances();
  }

  async getAllCashuMintBalances(): Promise<{ authorized: Record<string, number>, unauthorized: Record<string, number> }> {
    if (!this.walletService) return { authorized: {}, unauthorized: {} };
    
    return this.walletService.getAllMintBalances();
  }



  // Get cached balance immediately without async call
  getCachedBalance(): number {
    // Create wallet service if needed (to access cached balance)
    if (!this.walletService) {
      this.walletService = new CashuWalletService(this.ndk);
    }
    return this.walletService.getCachedBalance();
  }

  // Get cached balance for a specific user
  getCachedBalanceForUser(userPubkey: string): number {
    // Create wallet service if needed
    if (!this.walletService) {
      this.walletService = new CashuWalletService(this.ndk);
    }
    return this.walletService.loadCachedBalanceForUser(userPubkey);
  }

  // Subscribe to balance updates
  onBalanceUpdate(callback: (balance: number) => void): () => void {
    // Create wallet service if needed (to enable subscriptions)
    if (!this.walletService) {
      this.walletService = new CashuWalletService(this.ndk);
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

  async meltToLightning(invoice: string): Promise<{ paid: boolean; preimage?: string; fee?: number; error?: string }> {
    if (!this.walletService) {
      throw new Error("Wallet not initialized");
    }
    
    return this.walletService.meltToLightning(invoice);
  }


  async receiveTokens(token: string): Promise<{ proofs: any[], amount: number }> {
    if (!this.walletService) {
      throw new Error("Wallet not initialized");
    }
    
    const result = await this.walletService.receiveTokens(token);
    return { proofs: [], amount: result.amount };
  }


  // Send nutzap to an event
  async sendNutzapToEvent(eventId: string, amount: number, mint?: string): Promise<void> {
    if (!this.walletService) {
      throw new Error('Wallet not initialized')
    }

    // First, we need to find the event author's pubkey
    const filter = {
      ids: [eventId],
      limit: 1
    };
    
    const events = await this.ndk.fetchEvents(filter);
    if (events.size === 0) {
      throw new Error('Event not found');
    }
    
    const event = Array.from(events)[0];
    const authorPubkey = event.pubkey;
    
    // Fetch the author's 10019 event to get their nutzap relays
    const event10019 = await this.fetchUser10019(authorPubkey);
    const nutzapRelays = event10019 ? this.parseNutzapRelays(event10019) : null;

    await this.walletService.sendNutzapToEvent(eventId, amount, mint, nutzapRelays);
  }


  // Send nutzap to a user
  async sendNutzap(pubkey: string, amount: number, mint?: string): Promise<void> {
    if (!this.walletService) {
      throw new Error('Wallet not initialized')
    }

    // Fetch the user's 10019 event to get their nutzap relays
    const event10019 = await this.fetchUser10019(pubkey);
    const nutzapRelays = event10019 ? this.parseNutzapRelays(event10019) : null;

    await this.walletService.sendNutzap(pubkey, amount, mint, nutzapRelays);
  }

  // Public methods for fetching events and subscribing
  async fetchEvents(filter: any): Promise<any[]> {
    const events = await this.ndk.fetchEvents(filter)
    return Array.from(events)
  }

  async subscribe(filter: any, options?: any): Promise<any> {
    return this.ndk.subscribe(filter, options)
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
      
      const events = await this.profileNdk.fetchEvents(filter);
      if (events.size === 0) {
        const fallbackRelays = ["wss://relay.damus.io", "wss://relay.nos.social", "wss://relay.primal.net"];
        return fallbackRelays;
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
        return fallbackRelays;
      }
      
      return relays;
    } catch (error) {
      console.error("Failed to fetch user relays:", error);
      const fallbackRelays = ["wss://relay.damus.io", "wss://relay.nos.social"];
      return fallbackRelays;
    }
  }

  // Parse nutzap relays from kind:10019 event
  parseNutzapRelays(event10019: any): string[] {
    const relays: string[] = [];
    if (event10019?.tags) {
      event10019.tags.forEach((tag: string[]) => {
        if (tag[0] === 'relay' && tag[1]) {
          relays.push(tag[1]);
        }
      });
    }
    return relays;
  }

  // Initialize group write relay pool from member pubkeys
  async initializeGroupWriteRelays(memberPubkeys: string[]): Promise<void> {
    try {
      // Collect all unique write relays from all members
      const allRelays = new Set<string>();
      
      // Fetch relay lists for all members in parallel
      await Promise.all(memberPubkeys.map(async (pubkey) => {
        try {
          const relays = await this.getUserRelays(pubkey);
          relays.forEach(relay => allRelays.add(relay));
        } catch (err) {
          // Skip failed relay fetches
        }
      }));
      
      // Add some fallback relays to ensure we have coverage
      allRelays.add("wss://relay.damus.io");
      allRelays.add("wss://relay.nos.social");
      allRelays.add("wss://relay.primal.net");
      
      // Limit total relays to prevent connection exhaustion (max 10)
      const limitedRelays = Array.from(allRelays).slice(0, 10);
      
      // Only recreate if the relay set has changed significantly
      const newRelaySet = new Set(limitedRelays);
      const hasChanged = newRelaySet.size !== this.groupWriteRelays.size || 
                        !Array.from(newRelaySet).every(relay => this.groupWriteRelays.has(relay));
      
      if (!hasChanged && this.groupWriteNdk) {
        return; // No change needed
      }
      
      // Close existing group write NDK if it exists
      if (this.groupWriteNdk) {
        try {
          Array.from(this.groupWriteNdk.pool.relays.values()).forEach(relay => {
            try {
              relay.disconnect();
            } catch (err) {
              // Ignore disconnection errors
            }
          });
        } catch (err) {
          // Ignore cleanup errors
        }
      }
      
      // Create new group write NDK instance
      this.groupWriteNdk = new NDK({
        explicitRelayUrls: limitedRelays,
        signer: this.ndk.signer
      });
      
      this.groupWriteRelays = newRelaySet;
      
      // Connect with timeout
      const connectPromise = this.groupWriteNdk.connect();
      const timeoutPromise = new Promise((_, reject) => 
        setTimeout(() => reject(new Error("Group write relay connection timeout")), 5000)
      );
      
      try {
        await Promise.race([connectPromise, timeoutPromise]);
      } catch (err) {
        console.warn("Some group write relays failed to connect:", err);
        // Continue with partial connections
      }
    } catch (error) {
      console.error("Failed to initialize group write relays:", error);
    }
  }

  // Fetch user's kind:10019 event using group write relay pool
  async fetchUser10019(pubkey: string): Promise<any> {
    try {
      // Check cache first
      if (this.user10019Cache.has(pubkey)) {
        return this.user10019Cache.get(pubkey);
      }

      // Check if we're already fetching this user's 10019
      if (this.user10019FetchPromises.has(pubkey)) {
        return await this.user10019FetchPromises.get(pubkey);
      }

      // Create a promise for this fetch to prevent duplicate requests
      const fetchPromise = (async () => {
        try {
          // Use group write NDK if available, otherwise fall back to profile NDK
          const ndkToUse = this.groupWriteNdk || this.profileNdk;

          // Fetch kind:10019 event
          const filter = {
            kinds: [10019],
            authors: [pubkey],
            limit: 1
          };

          // Fetch from group write relays or profile relays
          const events = await ndkToUse.fetchEvents(filter);
          
          let result = null;
          if (events.size > 0) {
            result = Array.from(events)[0];
          }

          // Cache the result (even if null, to avoid repeated lookups)
          this.user10019Cache.set(pubkey, result);
          
          return result;
        } finally {
          // Clean up the fetch promise
          this.user10019FetchPromises.delete(pubkey);
        }
      })();

      // Store the promise to prevent duplicate fetches
      this.user10019FetchPromises.set(pubkey, fetchPromise);
      
      return await fetchPromise;
    } catch (error) {
      console.error("Failed to fetch user 10019:", error);
      // Still cache the failure to avoid repeated attempts
      this.user10019Cache.set(pubkey, null);
      return null;
    }
  }

  // Fetch multiple users' kind:10019 events efficiently using group write relay pool
  async fetchMultipleUsers10019(pubkeys: string[]): Promise<Map<string, any>> {
    const result = new Map<string, any>();
    const pubkeysToFetch: string[] = [];
    
    // Check cache first and collect pubkeys that need fetching
    for (const pubkey of pubkeys) {
      if (this.user10019Cache.has(pubkey)) {
        const cached = this.user10019Cache.get(pubkey);
        if (cached !== null) {
          result.set(pubkey, cached);
        }
      } else {
        pubkeysToFetch.push(pubkey);
      }
    }
    
    // If all were cached, return early
    if (pubkeysToFetch.length === 0) {
      return result;
    }
    
    try {
      // Use group write NDK if available, otherwise fall back to profile NDK
      const ndkToUse = this.groupWriteNdk || this.profileNdk;

      // Fetch all kind:10019 events in one query
      const filter = {
        kinds: [10019],
        authors: pubkeysToFetch,
        limit: pubkeysToFetch.length
      };

      const events = await ndkToUse.fetchEvents(filter);
      
      // Map events back to authors and cache them
      events.forEach(event => {
        result.set(event.pubkey, event);
        this.user10019Cache.set(event.pubkey, event);
      });
      
      // Cache the pubkeys that didn't have events as null
      pubkeysToFetch.forEach(pubkey => {
        if (!result.has(pubkey)) {
          this.user10019Cache.set(pubkey, null);
        }
      });
      
    } catch (error) {
      console.error("Failed to fetch multiple users' 10019 events:", error);
    }
    
    return result;
  }

  async connect() {
    try {
      await Promise.all([this.ndk.connect(), this.profileNdk.connect()]);
      
      // Create wallet service immediately after connection
      if (!this.walletService) {
        this.walletService = new CashuWalletService(this.ndk);
      }

      const relays = Array.from(this.ndk.pool.relays.values());
      const firstRelay = await Promise.race(
        relays.map(
          (relay) =>
            new Promise<NDKRelay>((resolve, reject) => {
              // Check if already ready (status 5 = READY)
              if (relay.status === 5) {
                resolve(relay);
                return;
              }

              // Handle connection states
              const handleStatus = () => {
                if (relay.status === 5) {
                  cleanup();
                  resolve(relay);
                }
              };

              // Handle errors
              const handleError = (err: Error) => {
                cleanup();
                reject(err);
              };

              // Setup event listeners
              relay.on("authed", () => {
                cleanup();
                resolve(relay);
              });
              relay.on("disconnect", () =>
                handleError(new Error("Relay disconnected"))
              );
              relay.on("auth:failed", (err) =>
                handleError(new Error(`Auth failed: ${err.message}`))
              );

              const interval = setInterval(handleStatus, 100);

              const cleanup = () => {
                clearInterval(interval);
                relay.removeAllListeners("authed");
                relay.removeAllListeners("disconnect");
                relay.removeAllListeners("auth:failed");
              };

              setTimeout(() => {
                cleanup();
                reject(
                  new Error("Connection timeout waiting for authentication")
                );
              }, 3000);
            })
        )
      );

      console.log(
        "Connected to relays:",
        relays.map((r) => ({
          url: r.url,
          status: r.status === firstRelay.status ? "ready" : r.status,
          connected: r.connected,
        }))
      );
    } catch (error) {
      throw new NostrGroupError(`Failed to connect: ${error}`);
    }
  }

  async disconnect() {
    try {
      // Close all relay connections from all NDK instances
      const groupRelays = Array.from(this.ndk.pool.relays.values());
      const profileRelays = Array.from(this.profileNdk.pool.relays.values());
      const groupWriteRelays = this.groupWriteNdk ? Array.from(this.groupWriteNdk.pool.relays.values()) : [];

      await Promise.all([
        ...groupRelays.map((relay) => relay.disconnect()),
        ...profileRelays.map((relay) => relay.disconnect()),
        ...groupWriteRelays.map((relay) => relay.disconnect()),
      ]);

      // Clear any subscriptions
      this.ndk.pool.removeAllListeners();
      this.profileNdk.pool.removeAllListeners();
      if (this.groupWriteNdk) {
        this.groupWriteNdk.pool.removeAllListeners();
        this.groupWriteNdk = null;
      }
      
      // Clear group write relay cache
      this.groupWriteRelays.clear();

    } catch (error) {
      console.error("Error during disconnect:", error);
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
      const readyRelays = Array.from(this.ndk.pool.relays.values()).filter(
        (r) => r.status === 5
      );

      if (readyRelays.length === 0) {
        throw new NostrGroupError(
          "Please ensure you are authenticated.",
          "No ready relays available"
        );
      }

      const ndkEvent = new NDKEvent(this.ndk);
      ndkEvent.kind = kind;
      ndkEvent.tags = tags;
      ndkEvent.content = content;
      await ndkEvent.sign();

      await ndkEvent.publish();

      return ndkEvent;
    } catch (error) {
      // If it's a NDKPublishError, we can get specific relay errors
      if (error instanceof NDKPublishError) {
        for (const [relay, err] of error.errors) {
          throw new NostrGroupError(err.message, relay.url);
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
      user.ndk = this.profileNdk; // Use the profile-specific NDK instance
      await user.fetchProfile();

      // Cache the profile
      if (user.profile) {
        this.profileCache.set(pubkey, user.profile);
      }

      return user.profile;
    } catch (error) {
      console.error("Failed to fetch profile:", error);
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
      console.error("Failed to check relay admin status:", error);
      return false;
    }
  }
}

export function hashGroup(group: Group): string {
  const { id, name, invites, joinRequests: join_requests, content } = group;
  return JSON.stringify({ id, name, invites, join_requests, content });
}
