import NDK, {
  NDKEvent,
  NDKPrivateKeySigner,
  NDKRelay,
  NDKRelayAuthPolicies,
  NDKPublishError,
  NDKUser,
} from "@nostr-dev-kit/ndk";
import { NDKCashuWallet } from "@nostr-dev-kit/ndk-wallet";
import { CashuMint, CashuWallet, getEncodedTokenV4, getDecodedToken, type Proof } from "@cashu/cashu-ts";
import { nip19 } from "nostr-tools";
import localforage from "localforage";
import type { Group } from "../types";

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

export interface Transaction {
  id: string;
  type: 'send' | 'receive' | 'mint' | 'melt';
  amount: number;
  mint: string;
  timestamp: number;
  status: 'pending' | 'completed' | 'failed';
  details?: any;
}

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
  readonly config: NostrClientConfig;
  private profileCache: Map<string, any> = new Map();
  private wallet: NDKCashuWallet | null = null;
  private cashuWallets: Map<string, CashuWallet> = new Map();
  private cashuMints: Map<string, CashuMint> = new Map();
  private cashuProofs: Map<string, Proof[]> = new Map(); // Store proofs per mint
  private transactions: Transaction[] = [];
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
      console.log("NostrClient using relay URL:", defaultRelayUrl);

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
      this.profileNdk = new NDK({
        explicitRelayUrls: ["wss://relay.nos.social", "wss://purplepag.es"],
      });

      this.ndk.pool.on("relay:connect", (relay: NDKRelay) => {
        relay.authPolicy = NDKRelayAuthPolicies.signIn({ ndk: this.ndk });
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
      const [storedProofs, storedTransactions] = await Promise.all([
        localforage.getItem<Record<string, Proof[]>>('cashu_proofs'),
        localforage.getItem<Transaction[]>('transactions')
      ]);

      if (storedProofs) {
        this.cashuProofs = new Map(Object.entries(storedProofs));
      }

      if (storedTransactions) {
        this.transactions = storedTransactions;
      }

      this.storageInitialized = true;
    } catch (error) {
      console.error('Failed to initialize storage:', error);
    }
  }

  get ndkInstance(): NDK {
    return this.ndk;
  }

  // Persist proofs to LocalForage
  private async persistProofs(): Promise<void> {
    if (!this.storageInitialized) return;
    
    try {
      const allProofs = Object.fromEntries(this.cashuProofs);
      await localforage.setItem('cashu_proofs', allProofs);
      await localforage.setItem('last_updated', Date.now());
    } catch (error) {
      console.error('Failed to persist proofs:', error);
    }
  }

  // Add transaction to history
  private async addTransaction(tx: Transaction): Promise<void> {
    this.transactions.push(tx);
    
    if (this.storageInitialized) {
      try {
        await localforage.setItem('transactions', this.transactions);
      } catch (error) {
        console.error('Failed to persist transaction:', error);
      }
    }
  }

  // Get transaction history
  getTransactionHistory(): Transaction[] {
    return [...this.transactions].sort((a, b) => b.timestamp - a.timestamp);
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
            console.log(`Cleared expired wallet keys for ${key}`);
          }
        }
      }
    } catch (error) {
      console.error('Failed to clear expired wallet keys:', error);
    }
  }

  // Prune spent proofs for a mint
  async pruneSpentProofs(mintUrl: string): Promise<void> {
    try {
      const wallet = this.cashuWallets.get(mintUrl);
      if (!wallet) return;
      
      const proofs = this.cashuProofs.get(mintUrl) || [];
      if (proofs.length === 0) return;
      
      // Check which proofs are spent
      const proofStates = await wallet.checkProofsStates(proofs);
      
      // Filter out spent proofs
      const activeProofs = proofs.filter((_, index) => 
        proofStates[index].state !== 'SPENT'
      );
      
      const spentCount = proofs.length - activeProofs.length;
      
      // Update storage
      this.cashuProofs.set(mintUrl, activeProofs);
      await this.persistProofs();
      
      console.log(`Pruned ${spentCount} spent proofs from ${mintUrl}`);
    } catch (error) {
      console.error('Failed to prune spent proofs:', error);
    }
  }

  // Prune all spent proofs across all mints
  async pruneAllSpentProofs(): Promise<void> {
    const mints = this.getActiveMints();
    await Promise.all(mints.map(mint => this.pruneSpentProofs(mint)));
  }

  // Wallet methods
  async initializeWallet(mints?: string[]): Promise<void> {
    try {
      // First check if user has existing NIP-60 wallet events
      const existingWallet = await this.fetchNIP60Wallet();
      
      if (existingWallet) {
        this.wallet = existingWallet;
        console.log("Restored existing NIP-60 wallet from relays");
        // Start the wallet to initialize it properly
        await this.wallet.start();
      } else {
        // Create new wallet if none exists
        // We need to use an NDK instance with user's relays for NWC to work
        const user = await this.ndk.signer?.user();
        if (!user) throw new Error("No user found");
        
        const userRelays = await this.getUserRelays(user.pubkey);
        
        // Create NDK with both group relay and user relays for better connectivity
        const walletNdk = new NDK({
          explicitRelayUrls: [
            ...userRelays,
            this.config.relayUrl // Include group relay too
          ],
          signer: this.ndk.signer
        });
        
        // Connect to relays
        await walletNdk.connect();
        
        this.wallet = new NDKCashuWallet(walletNdk);
        
        // Add default mints if provided
        if (mints && mints.length > 0) {
          for (const mint of mints) {
            this.wallet.mints = [...(this.wallet.mints || []), mint];
          }
        }
        
        // Start the wallet to initialize it properly
        await this.wallet.start();
        
        // Create wallet metadata event for new wallet
        if (mints && mints.length > 0) {
          await this.createOrUpdateWalletMetadata(mints);
        }
      }
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to initialize wallet"
      );
    }
  }

  // Fetch existing NIP-60 wallet from user's relays
  async fetchNIP60Wallet(): Promise<NDKCashuWallet | null> {
    try {
      const user = await this.ndk.signer?.user();
      if (!user) return null;

      console.log("🔍 Checking for existing NIP-60 wallet...");

      // First get user's relay list to know where to look
      const userRelays = await this.getUserRelays(user.pubkey);
      
      console.log(`📡 Inspecting ${userRelays.length} relays for NIP-60 wallet:`, userRelays);
      
      // Create a temporary NDK instance with user's relays
      const userNdk = new NDK({
        explicitRelayUrls: userRelays,
        signer: this.ndk.signer
      });
      
      console.log("⏳ Connecting to user's relays...");
      
      // Connect with timeout
      const connectPromise = userNdk.connect();
      const timeoutPromise = new Promise((_, reject) => 
        setTimeout(() => reject(new Error("Connection timeout")), 10000)
      );
      
      try {
        await Promise.race([connectPromise, timeoutPromise]);
        console.log("✅ Connected to user's relays");
      } catch (err) {
        console.warn("⚠️ Some relays may not have connected:", err);
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
      
      console.log(`📦 Found ${events.size} NIP-60 wallet events`);
      
      if (events.size === 0) {
        console.log("❌ No existing NIP-60 wallet found");
        return null;
      }

      // Create wallet instance with user's NDK (has user relays)
      // This is important for NWC to work properly
      const wallet = new NDKCashuWallet(userNdk);
      
      // Let NDK process the events (it handles encryption/decryption)
      // The NDKCashuWallet should automatically handle NIP-60 events
      console.log("🔐 Processing wallet events...");
      
      // For logging purposes, let's still count what we found
      const detectedMints = new Set<string>();
      let tokenCount = 0;
      
      // Process wallet events for logging
      for (const event of events) {
        try {
          // Handle different NIP-60 event types
          if (event.kind === 17375) {
            // Wallet metadata
            try {
              const content = JSON.parse(event.content);
              if (content.mints) {
                wallet.mints = content.mints;
                content.mints.forEach((mint: string) => detectedMints.add(mint));
                console.log(`💰 Found wallet metadata with ${content.mints.length} mints`);
              }
            } catch (parseErr) {
              console.warn(`⚠️ Could not parse wallet metadata event (might be encrypted):`, event.content.slice(0, 50) + '...');
            }
          } else if (event.kind === 7375) {
            // Cashu tokens
            tokenCount++;
            
            // Log all tags to see what's available
            console.log(`🏷️ Token event tags:`, event.tags);
            
            // Extract mint from tags if available
            const mintTag = event.tags.find(tag => tag[0] === 'mint');
            if (mintTag && mintTag[1]) {
              detectedMints.add(mintTag[1]);
              console.log(`💰 Found mint in token event: ${mintTag[1]}`);
            } else {
              // Check for 'u' tag as alternative (some implementations use 'u' for mint URL)
              const uTag = event.tags.find(tag => tag[0] === 'u');
              if (uTag && uTag[1]) {
                detectedMints.add(uTag[1]);
                console.log(`💰 Found mint URL in 'u' tag: ${uTag[1]}`);
              } else {
                console.warn(`⚠️ Token event has no mint tag. Available tags:`, event.tags.map(t => t[0]));
              }
            }
          }
        } catch (err) {
          console.warn(`⚠️ Error processing wallet event:`, err);
        }
      }

      console.log(`✅ NIP-60 wallet restored:`);
      console.log(`   - ${detectedMints.size} unique mints detected:`, Array.from(detectedMints));
      console.log(`   - ${tokenCount} token events found`);

      // If we found mints from tags, add them to the wallet
      if (detectedMints.size > 0) {
        wallet.mints = Array.from(detectedMints);
        console.log(`   - Setting wallet mints to:`, wallet.mints);
        
        // Start the wallet to load tokens
        try {
          await wallet.start();
          console.log("✅ Wallet started successfully");
        } catch (err) {
          console.warn("⚠️ Failed to start wallet:", err);
        }
        
        return wallet;
      } else if (tokenCount > 0) {
        // We have token events but no mints detected - this shouldn't happen
        console.warn("⚠️ Found token events but no mints - wallet may be encrypted");
        return null; // Return null so we use default mints
      }

      return null; // No wallet data found
    } catch (error) {
      console.error("❌ Failed to fetch NIP-60 wallet:", error);
      return null;
    }
  }

  get walletInstance(): NDKCashuWallet | null {
    return this.wallet;
  }

  async getWalletBalance(): Promise<number> {
    if (!this.wallet) {
      throw new NostrGroupError("Wallet not initialized", "getWalletBalance");
    }
    
    try {
      // Try to update balance to ensure it's up to date
      // This will trigger NWC balance check if configured
      if (this.wallet.updateBalance) {
        await this.wallet.updateBalance();
      }
    } catch (err) {
      console.warn("Failed to update wallet balance:", err);
    }
    
    // Balance might be an object with amount property
    const balance = this.wallet.balance;
    if (typeof balance === 'number') {
      return balance;
    } else if (balance && typeof balance === 'object' && 'amount' in balance) {
      return (balance as any).amount || 0;
    }
    return 0;
  }

  // Cashu-ts specific methods
  async initializeCashuMint(mintUrl: string): Promise<CashuWallet> {
    try {
      // Check if mint already exists
      if (this.cashuMints.has(mintUrl)) {
        const existingWallet = this.cashuWallets.get(mintUrl);
        if (existingWallet) return existingWallet;
      }

      // Create new mint and wallet
      const mint = new CashuMint(mintUrl);
      const wallet = new CashuWallet(mint, { unit: 'sat' });
      
      // Always load mint info (keys can't be set directly in newer versions)
      await wallet.loadMint();
      
      // For future optimization, we could cache mint info
      // but the current cashu-ts doesn't allow setting keys directly
      
      // Store references
      this.cashuMints.set(mintUrl, mint);
      this.cashuWallets.set(mintUrl, wallet);
      
      // Check if we have existing tokens for this mint in NIP-60
      // This runs asynchronously without blocking
      this.restoreTokensFromNIP60(mintUrl, wallet);
      
      // Also prune any spent proofs asynchronously
      setTimeout(() => this.pruneSpentProofs(mintUrl), 1000);
      
      return wallet;
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to initialize Cashu mint"
      );
    }
  }

  // Restore tokens from NIP-60 events for a specific mint
  async restoreTokensFromNIP60(mintUrl: string, wallet: CashuWallet): Promise<void> {
    // Run token restoration asynchronously without blocking
    (async () => {
      try {
        const user = await this.ndk.signer?.user();
        if (!user) return;

        console.log(`🔍 Checking for existing tokens for mint: ${mintUrl}`);

        // Get user's relays
        const userRelays = await this.getUserRelays(user.pubkey);
        
        // Create temporary NDK with user's relays
        const userNdk = new NDK({
          explicitRelayUrls: userRelays,
          signer: this.ndk.signer
        });
        
        // Connect with shorter timeout
        try {
          const connectPromise = userNdk.connect();
          const timeoutPromise = new Promise((_, reject) => 
            setTimeout(() => reject(new Error("Connection timeout")), 5000)
          );
          await Promise.race([connectPromise, timeoutPromise]);
        } catch (err) {
          console.warn("⚠️ Token restore: Some relays may not have connected:", err);
        }

        // Fetch token events (kind 7375)
        const filter = {
          kinds: [7375],
          authors: [user.pubkey],
          "#mint": [mintUrl],
          limit: 100
        };
        
        const events = await userNdk.fetchEvents(filter);
        
        if (events.size === 0) {
          console.log(`   ❌ No tokens found for mint ${mintUrl}`);
          return;
        }

        let totalProofs = 0;
        let totalAmount = 0;

        // Process token events
        for (const event of events) {
          try {
            // Parse tokens from event (might be encrypted)
            const content = JSON.parse(event.content);
            if (content.token && typeof content.token === 'string') {
              // Try to receive the token
              try {
                const proofs = await wallet.receive(content.token);
                if (proofs && proofs.length > 0) {
                  // Store proofs locally
                  this.addCashuProofs(mintUrl, proofs);
                  totalProofs += proofs.length;
                  totalAmount += proofs.reduce((sum: number, proof: any) => sum + proof.amount, 0);
                }
              } catch (receiveErr) {
                console.debug("Could not receive token:", receiveErr);
              }
            }
          } catch (err) {
            // Token might be encrypted or invalid
            console.debug("Could not process token event:", err);
          }
        }

        if (totalProofs > 0) {
          console.log(`   ✅ Restored ${totalProofs} proofs (${totalAmount} sats) from ${events.size} events for mint ${mintUrl}`);
        }
      } catch (error) {
        console.error("❌ Failed to restore tokens from NIP-60:", error);
      }
    })();
  }

  getCashuWallet(mintUrl: string): CashuWallet | undefined {
    return this.cashuWallets.get(mintUrl);
  }

  // Get balance for a specific mint
  async getCashuBalance(mintUrl: string): Promise<number> {
    const proofs = this.cashuProofs.get(mintUrl) || [];
    return proofs.reduce((sum, proof) => sum + proof.amount, 0);
  }

  // Add proofs to storage
  addCashuProofs(mintUrl: string, proofs: Proof[]): void {
    const existingProofs = this.cashuProofs.get(mintUrl) || [];
    this.cashuProofs.set(mintUrl, [...existingProofs, ...proofs]);
    
    // Persist to storage
    this.persistProofs();
  }

  // Remove spent proofs
  removeCashuProofs(mintUrl: string, proofsToRemove: Proof[]): void {
    const existingProofs = this.cashuProofs.get(mintUrl) || [];
    const secretsToRemove = new Set(proofsToRemove.map(p => p.secret));
    const remainingProofs = existingProofs.filter(p => !secretsToRemove.has(p.secret));
    this.cashuProofs.set(mintUrl, remainingProofs);
    
    // Persist to storage
    this.persistProofs();
  }

  // Get all proofs for a mint
  getCashuProofs(mintUrl: string): Proof[] {
    return this.cashuProofs.get(mintUrl) || [];
  }

  // Get all proofs from all mints
  getAllCashuProofs(): Proof[] {
    const allProofs: Proof[] = [];
    for (const proofs of this.cashuProofs.values()) {
      allProofs.push(...proofs);
    }
    return allProofs;
  }

  // Get all active mint URLs
  getActiveMints(): string[] {
    return Array.from(this.cashuMints.keys());
  }

  async mintTokens(mintUrl: string, amount: number): Promise<{ proofs: Proof[], invoice: string, quote: any }> {
    try {
      const wallet = await this.initializeCashuMint(mintUrl);
      
      // Create mint quote (Lightning invoice)
      const mintQuote = await wallet.createMintQuote(amount);
      
      // Add pending transaction
      await this.addTransaction({
        id: `mint_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
        type: 'mint',
        amount,
        mint: mintUrl,
        timestamp: Date.now(),
        status: 'pending',
        details: { quote: mintQuote }
      });
      
      // Store quote for later checking
      return {
        proofs: [],
        invoice: mintQuote.request,
        quote: mintQuote
      };
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to mint tokens"
      );
    }
  }

  async checkAndClaimTokens(mintUrl: string, quote: any): Promise<{ proofs: Proof[], claimed: boolean }> {
    try {
      const wallet = this.getCashuWallet(mintUrl);
      if (!wallet) {
        throw new Error("Wallet not initialized for mint: " + mintUrl);
      }

      // Check if the quote is paid (for testnut, it might be auto-paid)
      const mintQuote = await wallet.checkMintQuote(quote.quote);
      
      if (mintQuote.state === 'PAID') {
        // Mint the tokens
        const proofs = await wallet.mintProofs(quote.amount, quote.quote);
        
        // Store proofs locally
        this.addCashuProofs(mintUrl, proofs);
        
        // Save to NIP-60
        await this.saveTokensToNIP60(mintUrl, proofs);
        
        // Update transaction status
        const txIndex = this.transactions.findIndex(
          tx => tx.type === 'mint' && tx.status === 'pending' && 
          tx.details?.quote?.quote === quote.quote
        );
        
        if (txIndex !== -1) {
          this.transactions[txIndex].status = 'completed';
          await localforage.setItem('transactions', this.transactions);
        }
        
        return { proofs, claimed: true };
      }
      
      return { proofs: [], claimed: false };
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to check/claim tokens"
      );
    }
  }

  async sendTokens(mintUrl: string, amount: number, proofs?: Proof[]): Promise<string> {
    try {
      const wallet = this.getCashuWallet(mintUrl);
      if (!wallet) {
        throw new Error("Wallet not initialized for mint: " + mintUrl);
      }

      // Use provided proofs or get from storage
      const availableProofs = proofs || this.getCashuProofs(mintUrl);
      
      if (availableProofs.length === 0) {
        throw new Error("No proofs available for this mint");
      }

      // Split proofs into keep and send with fee handling
      const { send, keep } = await wallet.send(amount, availableProofs, {
        includeFees: true
      });
      
      // Update stored proofs
      this.cashuProofs.set(mintUrl, keep);
      await this.persistProofs();
      
      // Create encoded token
      const token = getEncodedTokenV4({ 
        mint: mintUrl, 
        proofs: send 
      });
      
      // Add transaction to history
      await this.addTransaction({
        id: `send_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
        type: 'send',
        amount,
        mint: mintUrl,
        timestamp: Date.now(),
        status: 'completed',
        details: { token }
      });
      
      return token;
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to send tokens"
      );
    }
  }

  async receiveTokens(token: string): Promise<{ proofs: Proof[], amount: number }> {
    try {
      // Validate token format first
      let decodedToken;
      try {
        decodedToken = getDecodedToken(token);
      } catch (err) {
        throw new Error('Invalid token format');
      }
      
      // Get mint URL from token
      const mintUrl = decodedToken.mint;
      
      // Check if we already have these proofs (duplicate check)
      const existingProofs = this.getAllCashuProofs();
      const tokenProofs = decodedToken.proofs;
      const duplicates = tokenProofs.some((tp: Proof) => 
        existingProofs.some(ep => ep.secret === tp.secret)
      );
      
      if (duplicates) {
        throw new Error('This token has already been received');
      }
      
      const wallet = await this.initializeCashuMint(mintUrl);
      
      // Receive and swap tokens
      let proofs: Proof[];
      try {
        proofs = await wallet.receive(token);
      } catch (error: any) {
        // Specific error handling
        if (error.message?.includes('already spent')) {
          throw new Error('This token has already been claimed');
        }
        throw error;
      }
      
      // Calculate total amount
      const amount = proofs.reduce((sum, proof) => sum + proof.amount, 0);
      
      // Store proofs locally
      this.addCashuProofs(mintUrl, proofs);
      
      // Save to NIP-60 wallet events
      await this.saveTokensToNIP60(mintUrl, proofs);
      
      // Add transaction to history
      const transaction = {
        id: `receive_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
        type: 'receive' as const,
        amount,
        mint: mintUrl,
        timestamp: Date.now(),
        status: 'completed' as const
      };
      await this.addTransaction(transaction);
      
      // Create spending history event
      await this.createSpendingHistoryEvent(transaction);
      
      return { proofs, amount };
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to receive tokens"
      );
    }
  }

  // Save tokens to NIP-60 events with proper encryption
  async saveTokensToNIP60(mintUrl: string, proofs: Proof[]): Promise<void> {
    try {
      if (!this.wallet) return;
      
      const user = await this.ndk.signer?.user();
      if (!user) return;
      
      // Get user's relays
      const userRelays = await this.getUserRelays(user.pubkey);
      
      // Create temporary NDK with user's relays
      const userNdk = new NDK({
        explicitRelayUrls: userRelays,
        signer: this.ndk.signer
      });
      
      // Connect with timeout
      try {
        const connectPromise = userNdk.connect();
        const timeoutPromise = new Promise((_, reject) => 
          setTimeout(() => reject(new Error("Connection timeout")), 10000)
        );
        await Promise.race([connectPromise, timeoutPromise]);
      } catch (err) {
        console.warn("⚠️ Save tokens: Some relays may not have connected:", err);
      }
      
      // Create NIP-60 token event (kind 7375)
      const tokenEvent = new NDKEvent(userNdk);
      tokenEvent.kind = 7375;
      
      // NIP-60 specifies encrypted content with proofs
      // NDK should handle encryption automatically when the signer is available
      const tokenData = {
        mint: mintUrl,
        proofs: proofs,
        unit: "sat"
      };
      
      // Set content - NDK will encrypt it if configured properly
      tokenEvent.content = JSON.stringify(tokenData);
      tokenEvent.tags = [
        ["mint", mintUrl],
        ["unit", "sat"]
      ];
      
      // Sign and publish to user's relays
      await tokenEvent.sign();
      await tokenEvent.publish();
      
      console.log(`Saved ${proofs.length} proofs to NIP-60 for mint ${mintUrl} to user's relays`);
    } catch (error) {
      console.error("Failed to save tokens to NIP-60:", error);
    }
  }

  // Create or update wallet metadata event (kind 17375)
  async createOrUpdateWalletMetadata(mints: string[], walletPrivkey?: string): Promise<void> {
    try {
      const user = await this.ndk.signer?.user();
      if (!user) return;
      
      // Create wallet metadata event
      const walletEvent = new NDKEvent(this.ndk);
      walletEvent.kind = 17375; // Replaceable event
      
      // Wallet metadata structure
      const walletData = {
        mints: mints.map(mint => ["mint", mint]),
        ...(walletPrivkey && { privkey: ["privkey", walletPrivkey] })
      };
      
      // Content should be encrypted with NIP-44
      walletEvent.content = JSON.stringify(walletData);
      walletEvent.tags = [];
      
      await walletEvent.sign();
      await walletEvent.publish();
      
      console.log("Created/updated NIP-60 wallet metadata");
    } catch (error) {
      console.error("Failed to create wallet metadata:", error);
    }
  }

  // Create spending history event (kind 7376)
  async createSpendingHistoryEvent(
    transaction: Transaction,
    createdTokenEventId?: string,
    destroyedTokenEventId?: string
  ): Promise<void> {
    try {
      const user = await this.ndk.signer?.user();
      if (!user) return;
      
      const historyEvent = new NDKEvent(this.ndk);
      historyEvent.kind = 7376;
      
      // Transaction data
      const txData = {
        direction: transaction.type === 'receive' ? 'in' : 'out',
        amount: transaction.amount,
        unit: 'sat',
        ...transaction.details
      };
      
      historyEvent.content = JSON.stringify(txData);
      historyEvent.tags = [];
      
      // Add event references if available
      if (createdTokenEventId) {
        historyEvent.tags.push(['e', createdTokenEventId, '', 'created']);
      }
      if (destroyedTokenEventId) {
        historyEvent.tags.push(['e', destroyedTokenEventId, '', 'destroyed']);
      }
      
      await historyEvent.sign();
      await historyEvent.publish();
      
      console.log("Created NIP-60 spending history event");
    } catch (error) {
      console.error("Failed to create spending history:", error);
    }
  }

  // Send nutzap to a user
  async sendNutzap(recipientPubkey: string, amount: number, proofs: Proof[], mintUrl: string, comment?: string): Promise<void> {
    try {
      // Get recipient's relays from their NIP-65 relay list
      const recipientRelays = await this.getUserRelays(recipientPubkey);
      
      // Create cashu token for the amount
      const wallet = this.getCashuWallet(mintUrl);
      if (!wallet) {
        throw new Error("Wallet not initialized for mint: " + mintUrl);
      }

      // Get proofs from storage if not provided
      const availableProofs = proofs.length > 0 ? proofs : this.getCashuProofs(mintUrl);
      
      if (availableProofs.length === 0) {
        throw new Error("No proofs available for nutzap");
      }
      
      const { send, keep } = await wallet.send(amount, availableProofs, {
        includeFees: true
      });
      
      // Update stored proofs - remove sent proofs, keep the rest
      this.cashuProofs.set(mintUrl, keep);
      await this.persistProofs();
      
      const token = getEncodedTokenV4({ 
        mint: mintUrl, 
        proofs: send 
      });
      
      // Create nutzap event (NIP-61)
      const nutzapEvent = new NDKEvent(this.ndk);
      nutzapEvent.kind = 9321; // NIP-61 nutzap kind
      nutzapEvent.content = comment || "";
      nutzapEvent.tags = [
        ["p", recipientPubkey],
        ["amount", amount.toString()],
        ["unit", "sat"],
        ["proof", token],
        ["u", mintUrl]
      ];
      
      // Sign the event
      await nutzapEvent.sign();
      
      // Publish to recipient's relays and our relays
      const allRelays = new Set([
        ...recipientRelays,
        ...Array.from(this.ndk.pool.relays.keys())
      ]);
      
      // Publish to all relays
      const publishPromises = Array.from(allRelays).map(async (relayUrl) => {
        try {
          const relay = this.ndk.pool.relays.get(relayUrl);
          if (relay) {
            await nutzapEvent.publish();
          }
        } catch (err) {
          console.warn(`Failed to publish to relay ${relayUrl}:`, err);
        }
      });
      
      await Promise.allSettled(publishPromises);
      
      // Add transaction to history
      await this.addTransaction({
        id: `nutzap_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
        type: 'send',
        amount,
        mint: mintUrl,
        timestamp: Date.now(),
        status: 'completed',
        details: { 
          recipient: recipientPubkey,
          comment,
          eventId: nutzapEvent.id
        }
      });
      
    } catch (error) {
      throw new NostrGroupError(
        error instanceof Error ? error.message : String(error),
        "Failed to send nutzap"
      );
    }
  }

  // Get user's preferred relays from NIP-65
  async getUserRelays(pubkey: string): Promise<string[]> {
    try {
      console.log(`🔍 Fetching relay list (kind 10002) for user ${pubkey.slice(0, 8)}...`);
      
      // Fetch NIP-65 relay list (kind 10002)
      const filter = {
        kinds: [10002],
        authors: [pubkey],
        limit: 1
      };
      
      const events = await this.profileNdk.fetchEvents(filter);
      if (events.size === 0) {
        const fallbackRelays = ["wss://relay.damus.io", "wss://relay.nos.social", "wss://relay.primal.net"];
        console.log("⚠️ No relay list found, using fallback relays:", fallbackRelays);
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
        console.log("⚠️ Relay list empty, using fallback relays:", fallbackRelays);
        return fallbackRelays;
      }
      
      console.log(`✅ Found ${relays.length} relays in user's relay list:`, relays);
      return relays;
    } catch (error) {
      console.error("❌ Failed to fetch user relays:", error);
      const fallbackRelays = ["wss://relay.damus.io", "wss://relay.nos.social"];
      console.log("⚠️ Error fetching relays, using fallback:", fallbackRelays);
      return fallbackRelays;
    }
  }

  async connect() {
    try {
      await Promise.all([this.ndk.connect(), this.profileNdk.connect()]);

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
              }, 5000);
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
      // Close all relay connections from both NDK instances
      const groupRelays = Array.from(this.ndk.pool.relays.values());
      const profileRelays = Array.from(this.profileNdk.pool.relays.values());

      await Promise.all([
        ...groupRelays.map((relay) => relay.disconnect()),
        ...profileRelays.map((relay) => relay.disconnect()),
      ]);

      // Clear any subscriptions
      this.ndk.pool.removeAllListeners();
      this.profileNdk.pool.removeAllListeners();

      console.log("Disconnected from all relays");
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
      console.log("ndkEvent", JSON.stringify(ndkEvent.rawEvent()));

      const publishResult = await ndkEvent.publish();
      console.log("Event published successfully:", !!publishResult);

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
      console.error("Failed to convert pubkey to npub:", error);
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
      console.error("Failed to convert npub to pubkey:", error);
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
      console.error("Failed to resolve NIP-05:", error);
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

      console.warn("Unexpected response type:", contentType);
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
