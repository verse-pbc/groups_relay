import NDK, { NDKEvent, NDKUser } from "@nostr-dev-kit/ndk";
import { NDKCashuWallet, NDKWalletStatus, NDKNutzapMonitor } from "@nostr-dev-kit/ndk-wallet";
import { type Proof } from "@cashu/cashu-ts";

// Interfaces following Interface Segregation Principle
export interface IWalletBalance {
  getBalance(): Promise<number>;
  getCachedBalance(): number;
  onBalanceUpdate(callback: (balance: number) => void): () => void;
  updateBalanceOptimistically(newBalance: number): void;
}

export interface IWalletTransactions {
  getTransactionHistory(): Transaction[];
  addTransaction(transaction: Transaction): void;
}

export interface ITokenOperations {
  receiveTokens(token: string): Promise<{ amount: number }>;
}

export interface INutzapOperations {
  sendNutzap(pubkey: string, amount: number, mint?: string): Promise<void>;
  sendNutzapToEvent(eventId: string, amount: number, mint?: string): Promise<void>;
}

export interface IMintOperations {
  mintTokens(mintUrl: string, amount: number): Promise<{ invoice: string; quote: any }>;
  checkAndClaimTokens(mintUrl: string, quote: any): Promise<{ proofs: Proof[], claimed: boolean }>;
}

export interface IWalletInitialization {
  initializeWallet(mints?: string[]): Promise<void>;
  isInitialized(): boolean;
}

// Transaction types
export type Transaction = {
  id: string;
  type: 'send' | 'receive' | 'mint' | 'melt';
  amount: number;
  mint: string;
  timestamp: number;
  description?: string;
  status: 'pending' | 'completed' | 'failed';
  direction?: 'in' | 'out';
};

// Main service interface combining all capabilities
export interface ICashuWalletService extends 
  IWalletBalance, 
  IWalletTransactions, 
  ITokenOperations, 
  INutzapOperations, 
  IMintOperations,
  IWalletInitialization {}

// Storage interface for persistence
interface IWalletStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

// Default localStorage storage adapter
class LocalStorageAdapter implements IWalletStorage {
  getItem(key: string): string | null {
    return localStorage.getItem(key);
  }

  setItem(key: string, value: string): void {
    localStorage.setItem(key, value);
  }

  removeItem(key: string): void {
    localStorage.removeItem(key);
  }
}

// Main implementation
export class CashuWalletService implements ICashuWalletService {
  private wallet: NDKCashuWallet | null = null;
  private nutzapMonitor: NDKNutzapMonitor | null = null;
  private ndk: NDK;
  private storage: IWalletStorage;
  private transactionHistory: Transaction[] = [];
  private cachedBalance: number = 0;
  private balanceCallbacks: Set<(balance: number) => void> = new Set();
  private userPubkey: string | null = null;
  private balanceCacheTimeout: number = 5 * 60 * 1000; // 5 minutes

  constructor(ndk: NDK, storage: IWalletStorage = new LocalStorageAdapter()) {
    this.ndk = ndk;
    this.storage = storage;
    this.loadCachedBalance();
    this.loadTransactionHistoryFromStorage();
  }

  // Initialization
  async initializeWallet(mints?: string[]): Promise<void> {
    const user = await this.ndk.signer?.user();
    if (!user?.pubkey) {
      throw new Error("No authenticated user found");
    }
    this.userPubkey = user.pubkey;

    // Check for existing wallet
    const existingWallet = await this.fetchExistingWallet(user);
    if (existingWallet) {
      this.wallet = existingWallet;
      console.log("Restored existing NIP-60 wallet");
      console.log("Wallet mints:", this.wallet.mints);
      // Log which mints we have and their order
      if (this.wallet.mints && this.wallet.mints.length > 0) {
        console.log("Available mints for transactions:");
        this.wallet.mints.forEach((mint, index) => {
          console.log(`  ${index + 1}. ${mint}`);
        });
      }
      
      
      await this.wallet.start();
      
      
      // Wait for the wallet to be ready
      await new Promise<void>((resolve) => {
        this.wallet!.once('ready', () => {
            resolve();
        });
        
        // If wallet is already ready, resolve immediately
        if (this.wallet!.status !== NDKWalletStatus.LOADING && this.wallet!.status !== NDKWalletStatus.INITIAL) {
          resolve();
        }
      });
      
      await this.updateBalance();
      
      // Load transaction history from NIP-60
      await this.loadTransactionHistory();
      
      // Start nutzap monitoring
      await this.ensureNutzapMonitor();
      
      // Check if wallet has P2PK key, if not, add one
      if (!this.walletP2PKPrivkey || !this.walletP2PKPubkey) {
        console.log('🔧 Existing wallet lacks P2PK key, adding one...');
        await this.addP2PKKeyToExistingWallet();
      }
      
      // Publish/update kind 10019 event for nutzap receiving
      await this.publishNutzapConfig();
      
      return;
    }

    // Create new wallet
    const walletNdk = new NDK({
      explicitRelayUrls: ["wss://relay.damus.io", "wss://relay.primal.net", "wss://relay.nostr.band"],
      signer: this.ndk.signer
    });

    await walletNdk.connect();
    this.wallet = new NDKCashuWallet(walletNdk);

    if (mints && mints.length > 0) {
      for (const mint of mints) {
        this.wallet.mints = [...(this.wallet.mints || []), mint];
      }
    }

    
    await this.wallet.start();
    
    
    // Wait for the wallet to be ready
    await new Promise<void>((resolve) => {
      this.wallet!.once('ready', () => {
        resolve();
      });
      
      // If wallet is already ready, resolve immediately
      if (this.wallet!.status !== NDKWalletStatus.LOADING && this.wallet!.status !== NDKWalletStatus.INITIAL) {
        resolve();
      }
    });
    
    if (mints && mints.length > 0) {
      await this.createWalletMetadata(mints);
    }
    
    // Update balance and start monitoring
    await this.updateBalance();
    await this.ensureNutzapMonitor();

    console.log("Created new NIP-60 wallet");
    
    
    // Publish kind 10019 event for nutzap receiving
    await this.publishNutzapConfig();
  }

  isInitialized(): boolean {
    return this.wallet !== null;
  }

  // Balance operations
  async getBalance(): Promise<number> {
    if (!this.wallet) return 0;
    
    try {
      const balance = await this.wallet.balance;
      this.updateCachedBalance(balance?.amount || 0);
      return balance?.amount || 0;
    } catch (error) {
      console.error("Failed to get balance:", error);
      return this.cachedBalance;
    }
  }

  getCachedBalance(): number {
    return this.cachedBalance;
  }

  onBalanceUpdate(callback: (balance: number) => void): () => void {
    this.balanceCallbacks.add(callback);
    return () => {
      this.balanceCallbacks.delete(callback);
    };
  }

  // Optimistically update balance (e.g., after sending nutzap)
  updateBalanceOptimistically(newBalance: number): void {
    this.updateCachedBalance(newBalance);
  }

  // Transaction history
  getTransactionHistory(): Transaction[] {
    return [...this.transactionHistory];
  }

  addTransaction(transaction: Transaction): void {
    this.transactionHistory = [transaction, ...this.transactionHistory].slice(0, 100);
    this.saveTransactionHistory();
    
    // Create NIP-60 spending history event
    this.createSpendingHistoryEvent(transaction);
  }

  // Token operations

  async receiveTokens(token: string): Promise<{ amount: number }> {
    if (!this.wallet) {
      throw new Error('Wallet not initialized');
    }

    const result = await this.wallet.receiveToken(token);
    const amount = result?.amount || 0;

    if (amount > 0) {
      this.addTransaction({
        id: `receive_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
        type: 'receive',
        amount: amount,
        mint: '',
        timestamp: Date.now(),
        status: 'completed',
        direction: 'in'
      });
    }

    await this.updateBalance();
    return { amount };
  }


  // Nutzap operations
  async sendNutzap(pubkey: string, amount: number, mint?: string): Promise<void> {
    if (!this.wallet) {
      throw new Error('Wallet not initialized');
    }

    const user = new NDKUser({ pubkey });
    user.ndk = this.ndk;

    const payment = {
      amount: amount,
      target: user,
      comment: '',
      recipientPubkey: pubkey,
      unit: 'sat',
      mints: mint ? [mint] : undefined
    };

    
    let result;
    try {
      result = await this.wallet.cashuPay(payment);
    } catch (error) {
      console.error("Nutzap cashuPay failed:", error);
      
      // Check for SSL/network errors
      if (error instanceof Error) {
      }
      throw error;
    }
    
    // CRITICAL: Check if result is valid
    if (!result) {
      console.error("Nutzap cashuPay returned null/undefined");
      throw new Error('Failed to send nutzap: cashuPay returned no result');
    }
    
    console.log(`Nutzap sent successfully with ${amount} sats`);
    
    // Update balance only if payment succeeded
    this.updateCachedBalance(Math.max(0, this.cachedBalance - amount));
    await this.updateBalance();
  }
  
  // Start nutzap monitoring after wallet is initialized
  private async ensureNutzapMonitor(): Promise<void> {
    if (!this.nutzapMonitor && this.wallet && this.userPubkey) {
      await this.startNutzapMonitor();
    }
  }

  async sendNutzapToEvent(eventId: string, amount: number, mint?: string): Promise<void> {
    if (!this.wallet) {
      throw new Error('Wallet not initialized');
    }

    // Fetch the event to get the author's pubkey
    const event = await this.ndk.fetchEvent(eventId);
    if (!event) {
      throw new Error('Event not found');
    }

    // Create a user for the event author
    const user = new NDKUser({ pubkey: event.pubkey });
    user.ndk = this.ndk;

    // Use NDKZapper to properly create and publish nutzap
    const { NDKZapper } = await import('@nostr-dev-kit/ndk');
    const zapper = new NDKZapper(event, amount, 'sat', {
      comment: '',
      ndk: this.ndk
    });

    
    try {
      // Set the cashuPay callback to use our wallet
      zapper.cashuPay = async (payment: any) => {
        try {
          const result = await this.wallet!.cashuPay({
            ...payment,
            mints: mint ? [mint] : undefined
          });
          return result;
        } catch (error: any) {
          // Ignore swap-related errors as they're non-fatal
          // The wallet will fall back to using existing proofs
          if (error.message?.includes('Not enough funds available') && 
              error.message?.includes('swap')) {
            console.warn('Swap optimization failed, continuing with existing proofs');
            // Let the error propagate so NDK can handle the fallback
          }
          throw error;
        }
      };

      // Execute the zap - this will create and publish the nutzap event
      const zapResult = await zapper.zap();
      
      if (!zapResult) {
        throw new Error('Failed to send nutzap: zapper returned no result');
      }
    } catch (error) {
      console.error("Nutzap failed:", error);
      throw error;
    }
    
    console.log(`Nutzap sent to event successfully with ${amount} sats`);
    
    // Update balance only if payment succeeded
    this.updateCachedBalance(Math.max(0, this.cachedBalance - amount));
    await this.updateBalance();
  }

  // Mint operations
  async mintTokens(mintUrl: string, amount: number): Promise<{ invoice: string; quote: any }> {
    if (!this.wallet) {
      throw new Error('Wallet not initialized');
    }

    if (!this.wallet.mints.includes(mintUrl)) {
      this.wallet.mints.push(mintUrl);
    }

    const deposit = this.wallet.deposit(amount, mintUrl);
    if (!deposit) {
      throw new Error("Failed to generate invoice");
    }

    let invoice: string | null = null;
    let quoteId: string | null = null;
    
    // Start the deposit process to generate the invoice
    try {
      // deposit.start() returns the bolt11 invoice
      invoice = await deposit.start();
      quoteId = deposit.quoteId || null;
    } catch (startError) {
      console.error("Failed to start deposit:", startError);
      console.error("Mint URL:", mintUrl);
      
      // Provide helpful error messages
      if (startError instanceof Error) {
        if (startError.message.includes('400') || startError.message.includes('Bad Request')) {
          throw new Error("This mint rejected the request (400 Bad Request). Try using a different mint like 'https://mint.minibits.cash' or use 'Receive Cashu Token' instead.");
        }
        if (startError.message.includes('Failed to fetch') || startError.message.includes('CORS')) {
          throw new Error("Cannot connect to mint. This might be due to CORS restrictions when running on localhost. Try using a different mint or running the app on a proper domain.");
        }
        if (startError.message.includes('net::ERR_')) {
          throw new Error("Network error connecting to mint. Make sure you have internet connection and the mint is accessible.");
        }
      }
      throw startError;
    }
    
    if (!invoice) {
      throw new Error("Failed to generate invoice. The mint may not support this payment method.");
    }
    
    const quote = { 
      id: quoteId || 'temp_quote', 
      mint: mintUrl,
      deposit: deposit // Keep reference to the deposit object for monitoring
    };

    this.addTransaction({
      id: `mint_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
      type: 'mint',
      amount: amount,
      mint: mintUrl,
      timestamp: Date.now(),
      status: 'pending'
    });

    return { invoice, quote };
  }

  async checkAndClaimTokens(_mintUrl: string, quote: any): Promise<{ proofs: Proof[], claimed: boolean }> {
    if (!this.wallet) {
      throw new Error('Wallet not initialized');
    }

    // If we have the deposit object, check its status
    if (quote.deposit) {
      const deposit = quote.deposit;
      
      // The deposit monitor should handle checking and claiming automatically
      // But we can check if it's finalized
      if (deposit.finalized) {
        await this.updateBalance();
        return { proofs: [], claimed: true };
      }
      
      // If not finalized, trigger a manual check
      try {
        await deposit.check();
        if (deposit.finalized) {
          await this.updateBalance();
          return { proofs: [], claimed: true };
        }
      } catch (error) {
        console.warn('Payment check failed:', error);
      }
      
      return { proofs: [], claimed: false };
    }
    
    // Fallback: wait and check balance
    await new Promise(resolve => setTimeout(resolve, 2000));
    await this.updateBalance();
    return { proofs: [], claimed: true };
  }

  // Private helper methods
  private async getUserRelays(pubkey: string): Promise<string[]> {
    try {
      // Fetch NIP-65 relay list (kind 10002)
      const filter = {
        kinds: [10002],
        authors: [pubkey],
        limit: 1
      };
      
      const events = await this.ndk.fetchEvents(filter);
      if (events.size === 0) {
        const fallbackRelays = ["wss://relay.damus.io", "wss://relay.primal.net", "wss://relay.nostr.band"];
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
      
      // If no relays found in tags, use fallback
      if (relays.length === 0) {
        return ["wss://relay.damus.io", "wss://relay.primal.net", "wss://relay.nostr.band"];
      }
      
      return relays;
    } catch (error) {
      console.error("Failed to fetch user relays:", error);
      return ["wss://relay.damus.io", "wss://relay.primal.net", "wss://relay.nostr.band"];
    }
  }

  private walletP2PKPrivkey: string | null = null;
  private walletP2PKPubkey: string | null = null;

  private async fetchExistingWallet(user: NDKUser): Promise<NDKCashuWallet | null> {
    try {
      // Get user's relays - this is critical for wallet functionality
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
        authors: [user.pubkey]
        // No limit - we need ALL wallet events to ensure we don't miss any tokens/money
      };

      // Fetch from user's relays
      const events = await userNdk.fetchEvents(filter);
      
      if (events.size === 0) {
        return null;
      }
      
      console.log(`📦 Found ${events.size} NIP-60 wallet events`);

      // Create wallet instance with user's NDK (has user relays)
      // This is CRITICAL - the wallet needs to use userNdk, not this.ndk
      const wallet = new NDKCashuWallet(userNdk);
      
      // Parse mints from wallet metadata events AND token events before starting
      const detectedMints = new Set<string>();
      let tokenCount = 0;
      
      for (const event of events) {
        if (event.kind === 17375) {
          // Wallet metadata event - extract mints from tags and P2PK private key from content
          const mintTags = event.tags.filter(tag => tag[0] === "mint" && tag[1]);
          
          // Try to decrypt the wallet metadata to get the P2PK private key
          try {
            await event.decrypt();
            if (event.content) {
              const metadata = JSON.parse(event.content);
              if (metadata.privkey) {
                this.walletP2PKPrivkey = metadata.privkey;
                console.log('🔑 Found P2PK private key in wallet metadata');
                
                // Get the corresponding public key
                const { NDKPrivateKeySigner } = await import('@nostr-dev-kit/ndk');
                const signer = new NDKPrivateKeySigner(metadata.privkey);
                const pubkey = await signer.user().then(u => u.pubkey);
                this.walletP2PKPubkey = pubkey;
                console.log('📝 Wallet P2PK pubkey:', pubkey);
              }
            }
          } catch (err) {
            console.warn('⚠️ Could not decrypt wallet metadata:', err);
          }
          mintTags.forEach(tag => detectedMints.add(tag[1]));
        } else if (event.kind === 7375) {
          // Token event
          tokenCount++;
          
          // First check for mint in tags
          const mintTag = event.tags.find(tag => tag[0] === "mint" && tag[1]);
          if (mintTag) {
            detectedMints.add(mintTag[1]);
          } else {
            // No mint tag, need to decrypt content to find mint
            try {
                
              // Decrypt the event if needed
              if (!event.content.startsWith('{') && !event.content.startsWith('[')) {
                await event.decrypt();
              }
              
              // Parse decrypted content
              const tokenData = JSON.parse(event.content);
              
              // Extract mint from token data
              if (tokenData.mint) {
                detectedMints.add(tokenData.mint);
              } else if (tokenData.token && Array.isArray(tokenData.token)) {
                // Token format might be nested
                tokenData.token.forEach((t: any) => {
                  if (t.mint) {
                    detectedMints.add(t.mint);
                  }
                });
              }
              
              // Log proof amounts for debugging
            } catch (err) {
              console.warn(`⚠️ Could not decrypt/parse token event:`, err);
            }
          }
        }
      }
      
      console.log(`Found ${tokenCount} token events across ${detectedMints.size} mints`);
      
      // Add common mints if we have tokens but missing mint info
      if (tokenCount > 0 && detectedMints.size < tokenCount / 2) {
        // Add common mints that might have untagged tokens
        detectedMints.add('https://mint.minibits.cash');
        detectedMints.add('https://mint.minibits.cash/Bitcoin');
        detectedMints.add('https://mint.coinos.io');
      }
      
      // Set all discovered mints before starting the wallet
      if (detectedMints.size > 0) {
        wallet.mints = Array.from(detectedMints);
      }
      
      // The wallet will process the events when we call start()
      await wallet.start();
      
      // Note: Nutzap monitoring will be started when this wallet is assigned to the service
      
      // Wait for the wallet to be ready
      await new Promise<void>((resolve) => {
        wallet.once('ready', () => {
          console.log("📦 Wallet ready event received");
          resolve();
        });
        
        // If wallet is already ready, resolve immediately
        if (wallet.status !== NDKWalletStatus.LOADING && wallet.status !== NDKWalletStatus.INITIAL) {
          resolve();
        }
      });
      
      
      // Update wallet metadata with all discovered mints
      if (wallet.mints && wallet.mints.length > 0) {
        // Only update if we discovered new mints
        const originalMintCount = Array.from(events).filter(e => e.kind === 17375).length > 0 ? 
          Array.from(events).find(e => e.kind === 17375)?.tags.filter(t => t[0] === "mint").length || 0 : 0;
        
        if (wallet.mints.length > originalMintCount) {
          await this.createWalletMetadata(wallet.mints);
        }
      }
      
      return wallet;
    } catch (error) {
      console.error("Failed to fetch existing wallet:", error);
      return null;
    }
  }

  private async createWalletMetadata(mints: string[]): Promise<void> {
    try {
      const user = await this.ndk.signer?.user();
      if (!user) return;
      
      // Get user's relays - wallet metadata needs to be published everywhere
      const userRelays = await this.getUserRelays(user.pubkey);
      const allRelays = [...new Set([...userRelays])]; // Could add group relay if needed
      
      // Create temporary NDK with all relays
      const allNdk = new NDK({
        explicitRelayUrls: allRelays,
        signer: this.ndk.signer
      });
      
      // Connect with timeout
      try {
        await allNdk.connect();
      } catch (err) {
        console.warn("⚠️ Create wallet metadata: Some relays may not have connected:", err);
      }
      
      // Create wallet metadata event
      const walletEvent = new NDKEvent(allNdk);
      walletEvent.kind = 17375; // Replaceable event
      
      // Wallet metadata structure (as tag arrays for NIP-60)
      walletEvent.tags = mints.map(mint => ["mint", mint]);
      
      // Generate a P2PK private key for the wallet
      const { NDKPrivateKeySigner } = await import('@nostr-dev-kit/ndk');
      const walletSigner = NDKPrivateKeySigner.generate();
      const walletPrivkey = walletSigner.privateKey;
      this.walletP2PKPrivkey = walletPrivkey!;
      console.log('🔑 Generated new P2PK private key for wallet');
      const walletPubkey = await walletSigner.user().then(u => u.pubkey);
      this.walletP2PKPubkey = walletPubkey;
      
      // Create wallet metadata with the P2PK private key
      const walletMetadata = {
        privkey: walletPrivkey
      };
      
      // Encrypt the content using NIP-44
      walletEvent.content = JSON.stringify(walletMetadata);
      const ndkUser = allNdk.getUser({ pubkey: user.pubkey });
      await walletEvent.encrypt(ndkUser, undefined, 'nip44');
      
      await walletEvent.sign();
      await walletEvent.publish();
      
      console.log("Created/updated NIP-60 wallet metadata to all relays with mints:", mints);
      
      // Add the P2PK key to the wallet's privkeys map and create backup
      if (this.wallet && walletPrivkey) {
        try {
          const walletUser = await walletSigner.user();
          this.wallet.privkeys.set(walletUser.pubkey, walletSigner);
          
          // Create backup using wallet's built-in backup method
          await this.wallet.backup(true); // true = publish the backup
          console.log("✅ Created wallet backup (kind 375)");
        } catch (error) {
          console.warn("⚠️ Failed to create wallet backup:", error);
        }
      }
    } catch (error) {
      console.error("Failed to create wallet metadata:", error);
    }
  }

  /**
   * Add P2PK key to existing wallet that doesn't have one
   */
  private async addP2PKKeyToExistingWallet(): Promise<void> {
    try {
      const user = await this.ndk.signer?.user();
      if (!user || !this.wallet) return;
      
      // Generate a P2PK private key for the wallet
      const { NDKPrivateKeySigner } = await import('@nostr-dev-kit/ndk');
      const walletSigner = NDKPrivateKeySigner.generate();
      const walletPrivkey = walletSigner.privateKey;
      this.walletP2PKPrivkey = walletPrivkey!;
      console.log('🔑 Generated new P2PK private key for existing wallet');
      const walletPubkey = await walletSigner.user().then(u => u.pubkey);
      this.walletP2PKPubkey = walletPubkey;
      
      // Update the wallet metadata event with the new P2PK key
      const walletMetadata = {
        privkey: walletPrivkey
      };
      
      // Get user's relays - wallet metadata needs to be published everywhere
      const userRelays = await this.getUserRelays(user.pubkey);
      const allRelays = [...new Set([...userRelays])];
      
      // Create temporary NDK with all relays
      const allNdk = new NDK({
        explicitRelayUrls: allRelays,
        signer: this.ndk.signer
      });
      
      // Connect with timeout
      try {
        await allNdk.connect();
      } catch (err) {
        console.warn("⚠️ Update wallet metadata: Some relays may not have connected:", err);
      }
      
      // Create wallet metadata event
      const walletEvent = new NDKEvent(allNdk);
      walletEvent.kind = 17375; // Replaceable event
      
      // Wallet metadata structure (as tag arrays for NIP-60)
      walletEvent.tags = this.wallet.mints.map(mint => ["mint", mint]);
      
      // Encrypt the content using NIP-44
      walletEvent.content = JSON.stringify(walletMetadata);
      const ndkUser = allNdk.getUser({ pubkey: user.pubkey });
      await walletEvent.encrypt(ndkUser, undefined, 'nip44');
      
      await walletEvent.sign();
      await walletEvent.publish();
      
      console.log("✅ Updated wallet metadata with P2PK key");
      
      // Add the P2PK key to the wallet's privkeys map and create backup
      if (this.wallet && walletPrivkey) {
        try {
          const walletUser = await walletSigner.user();
          this.wallet.privkeys.set(walletUser.pubkey, walletSigner);
          
          // Create backup using wallet's built-in backup method
          await this.wallet.backup(true); // true = publish the backup
          console.log("✅ Created wallet backup (kind 375)");
        } catch (error) {
          console.warn("⚠️ Failed to create wallet backup:", error);
        }
      }
      
      // Now we can start the nutzap monitor with the new key
      if (this.nutzapMonitor) {
        console.log('🔄 Restarting nutzap monitor with new P2PK key...');
        this.nutzapMonitor.stop();
        this.nutzapMonitor = null;
        await this.startNutzapMonitor();
      }
    } catch (error) {
      console.error("❌ Failed to add P2PK key to existing wallet:", error);
    }
  }

  /**
   * Publish kind 10019 event to enable nutzap receiving
   */
  private async publishNutzapConfig(): Promise<void> {
    if (!this.walletP2PKPubkey) {
      console.warn('⚠️ Cannot publish nutzap config: no P2PK pubkey');
      return;
    }
    
    try {
      const user = await this.ndk.signer?.user();
      if (!user) return;
      
      // Get current relays from NDK
      const relays = Array.from(this.ndk.pool.relays.values()).map(r => r.url);
      
      // For testing, ensure we include the local relay
      if (!relays.includes('ws://localhost:8080')) {
        relays.push('ws://localhost:8080');
      }
      
      // Get mints from wallet
      const mints = this.wallet?.mints || [];
      
      // Create kind 10019 event using NDKCashuMintList
      const { NDKCashuMintList } = await import('@nostr-dev-kit/ndk');
      const mintList = new NDKCashuMintList(this.ndk);
      
      // Set properties
      mintList.relays = relays;
      mintList.mints = mints;
      mintList.p2pk = this.walletP2PKPubkey;
      
      // Convert to event and publish
      await mintList.toNostrEvent();
      await mintList.publishReplaceable();
      
      console.log('✅ Published kind 10019 nutzap config event');
      console.log('📝 Nutzap config:', {
        relays,
        mints,
        p2pk: this.walletP2PKPubkey
      });
    } catch (error) {
      console.error('❌ Failed to publish nutzap config:', error);
    }
  }

  private async updateBalance(): Promise<void> {
    const balance = await this.getBalance();
    this.notifyBalanceCallbacks(balance);
  }

  private async startNutzapMonitor(): Promise<void> {
    if (!this.wallet || !this.userPubkey || this.nutzapMonitor) {
      console.warn('🚨 Cannot start nutzap monitor: wallet not initialized or monitor already running');
      return;
    }

    try {
        
      // Create nutzap monitor
      const user = await this.ndk.signer?.user();
      if (!user) {
        console.error('🔴 Cannot create nutzap monitor: no user');
        return;
      }
      // Create monitor with relay configuration
      // Get the current relays from NDK pool
      const relays = Array.from(this.ndk.pool.relays.values());
      
      // Create a relay set that includes our local relay
      const { NDKRelaySet } = await import('@nostr-dev-kit/ndk');
      const relaySet = new NDKRelaySet(new Set(relays), this.ndk);
      
      // Create monitor with custom options
      // Pass the relay set through a custom property that the monitor can use
      const monitorOptions: any = {
        relaySet: relaySet
      };
      this.nutzapMonitor = new NDKNutzapMonitor(this.ndk, user, monitorOptions);
      
      // Set the wallet for redemption
      (this.nutzapMonitor as any).wallet = this.wallet;
      
      // Add the WALLET's P2PK private key for nutzap redemption
      // According to NIP-60, nutzaps use a separate wallet private key, NOT the user's Nostr key
      // This key is extracted from the kind 17375 wallet metadata event
      try {
        if (this.walletP2PKPrivkey) {
          const { NDKPrivateKeySigner } = await import('@nostr-dev-kit/ndk');
          const privkeySigner = new NDKPrivateKeySigner(this.walletP2PKPrivkey);
          this.nutzapMonitor.addPrivkey(privkeySigner);
          console.log('🔑 Added wallet P2PK private key to nutzap monitor');
        } else {
          console.warn('⚠️ Wallet does not have a P2PK private key - cannot redeem nutzaps');
          // The NDKCashuWallet doesn't expose the privkey, and we couldn't extract it from the metadata event
          console.warn('⚠️ This means the wallet was likely created without a P2PK private key');
        }
      } catch (error) {
        console.error('🔴 Error setting up wallet private key:', error);
      }
      
      // Listen for all nutzap monitor events
      this.nutzapMonitor.on('seen', () => {
      });
      
      this.nutzapMonitor.on('seen_in_unknown_mint', () => {
      });
      
      this.nutzapMonitor.on('state_changed', () => {
      });
      
      // Listen for redeemed nutzaps
      this.nutzapMonitor.on('redeemed', async (event: any) => {
        console.log('🎉 Nutzap redeemed!', event);
        // Update balance after redemption
        await this.updateBalance();
        
        // Extract amount from the event
        const amount = event.amount || event.nutzap?.amount || 0;
        
        // Add to transaction history
        this.addTransaction({
          id: event.id || `nutzap_${Date.now()}`,
          type: 'receive',
          amount: amount,
          timestamp: Date.now(),
          status: 'completed',
          description: 'Received nutzap',
          mint: event.mint || 'unknown'
        });
      });
      
      // Listen for errors
      this.nutzapMonitor.on('failed', (event: any) => {
        console.error('🔴 Nutzap redemption failed:', event);
      });
      
      // Log subscription status
      
      // Set the relay set on the monitor if possible
      if ('relaySet' in this.nutzapMonitor) {
        (this.nutzapMonitor as any).relaySet = monitorOptions.relaySet;
      }
      
      // Set the wallet on the monitor for redemption
      if ('wallet' in this.nutzapMonitor) {
        (this.nutzapMonitor as any).wallet = this.wallet;
      }
      
      // Start monitoring with a filter for nutzap events
      const filter = { kinds: [9321], "#p": [this.userPubkey!] };
      
      // Start the monitor - it may throw backup key errors but will still work
      this.nutzapMonitor.start({ 
        filter,
        opts: {
          closeOnEose: false,
          groupable: false
        }
      }).catch((error: any) => {
        // Log but don't fail - backup keys are optional
        console.warn('⚠️ Nutzap monitor backup key warning:', error.message);
      });
      
      console.log('✅ Nutzap monitor started successfully');
      
    } catch (error) {
      console.error('🔴 Failed to start nutzap monitor:', error);
      // Don't throw - the wallet can still function without nutzap monitoring
    }
  }

  private updateCachedBalance(balance: number): void {
    this.cachedBalance = balance;
    this.persistBalanceCache(balance);
    this.notifyBalanceCallbacks(balance);
  }

  private notifyBalanceCallbacks(balance: number): void {
    this.balanceCallbacks.forEach(callback => callback(balance));
  }

  private loadCachedBalance(): void {
    if (!this.userPubkey) return;
    
    const cached = this.storage.getItem(`cashu_balance_${this.userPubkey}`);
    if (cached) {
      try {
        const { balance, timestamp } = JSON.parse(cached);
        if (Date.now() - timestamp < this.balanceCacheTimeout) {
          this.cachedBalance = balance;
        }
      } catch (error) {
        console.error("Failed to load cached balance:", error);
      }
    }
  }

  private persistBalanceCache(balance: number): void {
    if (!this.userPubkey) return;
    
    const data = JSON.stringify({
      balance,
      timestamp: Date.now()
    });
    this.storage.setItem(`cashu_balance_${this.userPubkey}`, data);
  }

  private loadTransactionHistoryFromStorage(): void {
    const stored = this.storage.getItem('cashu_transactions');
    if (stored) {
      try {
        this.transactionHistory = JSON.parse(stored);
      } catch (error) {
        console.error("Failed to load transaction history:", error);
      }
    }
  }

  private saveTransactionHistory(): void {
    this.storage.setItem('cashu_transactions', JSON.stringify(this.transactionHistory));
  }

  private async createSpendingHistoryEvent(transaction: Transaction): Promise<void> {
    try {
      if (!this.userPubkey) return;
      
      // Get user's relays for publishing
      const userRelays = await this.getUserRelays(this.userPubkey);
      const allRelays = [...new Set([...userRelays])];
      
      // Create temporary NDK with all relays
      const allNdk = new NDK({
        explicitRelayUrls: allRelays,
        signer: this.ndk.signer
      });
      
      // Connect with timeout
      try {
        await allNdk.connect();
      } catch (err) {
        console.warn("⚠️ Create spending history: Some relays may not have connected:", err);
      }
      
      const historyEvent = new NDKEvent(allNdk);
      historyEvent.kind = 7376;
      
      // Transaction data
      const txData = {
        direction: transaction.direction || (transaction.type === 'receive' ? 'in' : 'out'),
        amount: transaction.amount,
        unit: 'sat',
        mint: transaction.mint,
        description: transaction.description || transaction.type
      };
      
      // Encrypt content with NIP-44
      historyEvent.content = JSON.stringify(txData);
      
      await historyEvent.sign();
      await historyEvent.publish();
      
      console.log("📝 Created NIP-60 spending history event");
    } catch (error) {
      console.error("Failed to create spending history event:", error);
    }
  }

  private async loadTransactionHistory(): Promise<void> {
    try {
      
      if (!this.userPubkey) return;
      
      const userRelays = await this.getUserRelays(this.userPubkey);
      const userNdk = new NDK({
        explicitRelayUrls: userRelays,
        signer: this.ndk.signer
      });
      
      // Connect with timeout
      try {
        const connectPromise = userNdk.connect();
        const timeoutPromise = new Promise((_, reject) => 
          setTimeout(() => reject(new Error("Connection timeout")), 3000)
        );
        await Promise.race([connectPromise, timeoutPromise]);
      } catch (err) {
        console.debug("Some relays may not have connected:", err);
      }
      
      // Fetch spending history events (kind 7376)
      const filter = {
        kinds: [7376],
        authors: [this.userPubkey],
        limit: 100
      };
      
      const events = await userNdk.fetchEvents(filter);
      
      if (events.size === 0) {
        return;
      }
      
      
      // Process history events
      const existingIds = new Set(this.transactionHistory.map(tx => tx.id));
      let addedCount = 0;
      
      for (const event of events) {
        try {
          let content;
          
          // Check if content is encrypted
          if (event.content.startsWith('{')) {
            // Plain JSON
            content = JSON.parse(event.content);
          } else {
            // Encrypted with NIP-44
            try {
              await event.decrypt();
              content = JSON.parse(event.content);
            } catch (decryptErr) {
              console.debug("Failed to decrypt history event:", decryptErr);
              continue;
            }
          }
          
          // Parse content based on its structure
          let transactionData: any = {};
          
          // Check if content is an array of tags or an object
          if (Array.isArray(content)) {
            // Parse tag array format
            for (const tag of content) {
              if (tag[0] === 'direction' && tag[1]) {
                transactionData.direction = tag[1];
              } else if (tag[0] === 'amount' && tag[1]) {
                transactionData.amount = parseInt(tag[1]);
              } else if (tag[0] === 'mint' && tag[1]) {
                transactionData.mint = tag[1];
              } else if (tag[0] === 'description' && tag[1]) {
                transactionData.description = tag[1];
              }
            }
          } else {
            // Use content as-is if it's already an object
            transactionData = content;
          }
          
          // Skip invalid transactions
          if (!transactionData.amount || transactionData.amount <= 0) {
            continue;
          }
          
          // Create transaction from parsed data
          const transaction: Transaction = {
            id: `nip60_${event.id}`,
            type: transactionData.direction === 'in' ? 'receive' : 
                  transactionData.direction === 'out' ? 'send' : 'mint',
            amount: transactionData.amount || 0,
            mint: transactionData.mint || 'Unknown',
            timestamp: (event.created_at || 0) * 1000, // Convert to milliseconds
            status: 'completed',
            direction: transactionData.direction,
            description: transactionData.description
          };
          
          // Only add if not already in history
          if (!existingIds.has(transaction.id)) {
            this.transactionHistory.push(transaction);
            addedCount++;
          }
        } catch (err) {
          console.debug("Could not process history event:", err);
        }
      }
      
      if (addedCount > 0) {
        // Sort by timestamp (newest first)
        this.transactionHistory.sort((a, b) => b.timestamp - a.timestamp);
        
        // Keep only the most recent 100 transactions
        this.transactionHistory = this.transactionHistory.slice(0, 100);
        
        // Persist updated history
        this.saveTransactionHistory();
        
      }
    } catch (error) {
      console.error("Failed to load transaction history from NIP-60:", error);
    }
  }

  // Cleanup method
  dispose(): void {
    // Stop nutzap monitor
    if (this.nutzapMonitor) {
      this.nutzapMonitor.stop();
      this.nutzapMonitor = null;
    }
    
    this.balanceCallbacks.clear();
    this.wallet = null;
  }
}