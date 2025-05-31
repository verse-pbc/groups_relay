import NDK, { 
  NDKEvent, 
  NDKUser, 
  NDKCashuMintList,
  NDKZapper,
  NDKPrivateKeySigner,
  NDKRelaySet
} from "@nostr-dev-kit/ndk";
import {
  NDKCashuWallet,
  NDKWalletStatus,
  NDKNutzapMonitor,
} from "@nostr-dev-kit/ndk-wallet";
import { type Proof } from "@cashu/cashu-ts";

// Interfaces following Interface Segregation Principle
export interface IWalletBalance {
  getBalance(): Promise<number>;
  getBalanceForRecipient(recipientPubkey: string): Promise<number>;
  getMintBalances(): Promise<Record<string, number>>;
  getAllMintBalances(): Promise<{
    authorized: Record<string, number>;
    unauthorized: Record<string, number>;
  }>;
  getCachedBalance(): number;
  loadCachedBalanceForUser(userPubkey: string): number;
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
  sendNutzap(
    pubkey: string,
    amount: number,
    mint?: string,
    nutzapRelays?: string[] | null
  ): Promise<void>;
  sendNutzapToEvent(
    eventId: string,
    amount: number,
    mint?: string,
    nutzapRelays?: string[] | null
  ): Promise<void>;
}

export interface IMintOperations {
  mintTokens(
    mintUrl: string,
    amount: number
  ): Promise<{ invoice: string; quote: any }>;
  checkAndClaimTokens(
    mintUrl: string,
    quote: any
  ): Promise<{ proofs: Proof[]; claimed: boolean }>;
  meltToLightning(invoice: string): Promise<{
    paid: boolean;
    preimage?: string;
    fee?: number;
    error?: string;
  }>;
  addMint(mintUrl: string): Promise<void>;
  removeMint(mintUrl: string): Promise<void>;
  publishNutzapConfig(): Promise<void>;
}

export interface IWalletInitialization {
  initializeWallet(mints?: string[]): Promise<void>;
  isInitialized(): boolean;
}

// Transaction types
export type Transaction = {
  id: string;
  type: "send" | "receive" | "mint" | "melt";
  amount: number;
  mint: string;
  timestamp: number;
  description?: string;
  status: "pending" | "completed" | "failed";
  direction?: "in" | "out";
};

// Cashu event parsing interface
export interface ICashuEventParsing {
  parseNutzapRelays(event10019: any): string[];
  parseNutzapMints(event10019: any): string[];
  parseNutzapP2PK(event10019: any): string | null;
  fetchUser10019(pubkey: string): Promise<any>;
  fetchMultipleUsers10019(pubkeys: string[]): Promise<Map<string, any>>;
}

// Main service interface combining all capabilities
export interface ICashuWalletService
  extends IWalletBalance,
    IWalletTransactions,
    ITokenOperations,
    INutzapOperations,
    IMintOperations,
    IWalletInitialization,
    ICashuEventParsing {}

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

// Main implementation - Cashu Wallet Service using NDK
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
  private lastBalanceCalculationTime = 0;
  private user10019Cache: Map<string, any> = new Map();
  private user10019FetchPromises: Map<string, Promise<any>> = new Map(); // Prevent duplicate fetches

  constructor(ndk: NDK, storage: IWalletStorage = new LocalStorageAdapter()) {
    this.ndk = ndk;
    this.storage = storage;
    this.loadCachedBalance();
    this.loadTransactionHistoryFromStorage();
    
    // Set up wallet event listeners when wallet is initialized
    this.setupWalletEventListeners();
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
        this.wallet!.once("ready", () => {
          resolve();
        });

        // If wallet is already ready, resolve immediately
        if (
          this.wallet!.status !== NDKWalletStatus.LOADING &&
          this.wallet!.status !== NDKWalletStatus.INITIAL
        ) {
          resolve();
        }
      });

      // Set up NDK's wallet integration for automatic zapper usage
      this.ndk.wallet = this.wallet;
      console.log("\u2705 Set up NDK wallet integration for automatic zapper usage");

      await this.updateBalance();

      // Load transaction history from NIP-60
      await this.loadTransactionHistory();

      // Start nutzap monitoring
      await this.ensureNutzapMonitor();

      // Check if wallet has P2PK key, if not, add one
      if (!this.walletP2PKPrivkey || !this.walletP2PKPubkey) {
        console.log("🔧 Existing wallet lacks P2PK key, adding one...");
        await this.addP2PKKeyToExistingWallet();
      }

      // Set up NDK's wallet integration for automatic zapper usage
      this.ndk.wallet = this.wallet;
      console.log("\u2705 Set up NDK wallet integration for existing wallet");

      // Publish/update kind 10019 event for nutzap receiving
      await this.publishNutzapConfig();

      return;
    }

    // Create new wallet
    const walletNdk = new NDK({
      // TODO: initialize with the user relays, not these hardcoded values unless we couldn't find relays
      explicitRelayUrls: [
        "wss://relay.damus.io",
        "wss://relay.primal.net",
        "wss://relay.nostr.band",
      ],
      signer: this.ndk.signer,
    });

    await walletNdk.connect();
    this.wallet = new NDKCashuWallet(walletNdk);

    // Note: walletNdk is now owned by the wallet and will be cleaned up when wallet is disposed

    if (mints && mints.length > 0) {
      for (const mint of mints) {
        this.wallet.mints = [...(this.wallet.mints || []), mint];
      }
    }

    await this.wallet.start();

    // Wait for the wallet to be ready
    await new Promise<void>((resolve) => {
      this.wallet!.once("ready", () => {
        resolve();
      });

      // If wallet is already ready, resolve immediately
      if (
        this.wallet!.status !== NDKWalletStatus.LOADING &&
        this.wallet!.status !== NDKWalletStatus.INITIAL
      ) {
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

  // Balance operations - use NDK's built-in balance methods with state access
  async getBalance(): Promise<number> {
    if (!this.wallet) return 0;

    try {
      // Use throttling to prevent excessive recalculation
      const now = Date.now();
      if (now - this.lastBalanceCalculationTime < 1000) {
        // Max once per second
        console.log(
          `  🔄 Using recently calculated balance: ${this.cachedBalance}`
        );
        return this.cachedBalance;
      }

      // Use NDK's built-in balance getter which uses wallet.state internally
      const balance = this.wallet.balance?.amount || 0;

      this.lastBalanceCalculationTime = now;
      this.updateCachedBalance(balance);
      return balance;
    } catch (error) {
      console.error("Failed to get balance:", error);
      return this.cachedBalance;
    }
  }

  /**
   * Get detailed wallet state information using NDK's state management
   */
  getWalletState() {
    if (!this.wallet) return null;
    
    return {
      status: this.wallet.status,
      mints: this.wallet.mints,
      balance: this.wallet.balance,
      mintBalances: this.wallet.mintBalances,
      // Access detailed state information
      detailedBalance: this.wallet.state?.getBalance({ onlyAvailable: true }) || 0,
      reservedBalance: this.wallet.state?.getBalance({ onlyAvailable: false }) || 0,
      totalProofs: this.wallet.state?.tokens.size || 0
    };
  }


  /**
   * Get balance available for sending to a specific recipient using NDK methods
   * This checks which mints both sender and recipient have in common
   */
  async getBalanceForRecipient(recipientPubkey: string): Promise<number> {
    if (!this.wallet) return 0;

    try {
      // Use NDK's built-in mintBalances getter
      const mintBalances = this.wallet.mintBalances || {};

      // Get the recipient's accepted mints from their kind:10019 event
      const recipientMints =
        await this.getRecipientAcceptedMints(recipientPubkey);
      console.log("🔍 Debug getBalanceForRecipient (using NDK methods):");
      console.log("  NDK mint balances:", mintBalances);
      console.log("  Recipient accepted mints:", recipientMints);

      // Only count balance from mints the recipient accepts
      let availableBalance = 0;
      for (const [mint, balance] of Object.entries(mintBalances)) {
        if (recipientMints.includes(mint)) {
          availableBalance += balance;
          console.log(
            `  ✅ Can send ${balance} sats from shared mint: ${mint}`
          );
        } else {
          console.warn(
            `  ⚠️ Cannot send ${balance} sats from unaccepted mint: ${mint}`
          );
        }
      }

      console.log(`  Total sendable balance to recipient: ${availableBalance}`);
      return availableBalance;
    } catch (error) {
      console.error("Failed to get balance for recipient:", error);
      return 0;
    }
  }

  /**
   * Get mints that a recipient accepts from their kind:10019 event using NDKCashuMintList
   */
  private async getRecipientAcceptedMints(
    recipientPubkey: string
  ): Promise<string[]> {
    try {
      const filter = {
        kinds: [10019],
        authors: [recipientPubkey],
        limit: 1,
      };

      const events = await this.ndk.fetchEvents(filter);
      if (events.size === 0) return [];

      const event = Array.from(events)[0];
      
      // Use NDKCashuMintList for cleaner parsing
      const mintList = NDKCashuMintList.from(event);
      return mintList.mints; // Returns deduplicated array
      
    } catch (error) {
      console.error("Failed to fetch recipient accepted mints:", error);
      return [];
    }
  }

  /**
   * Get balance per mint using NDK's built-in mintBalances getter and state access
   * @returns Map of mint URL to balance in sats
   */
  async getMintBalances(): Promise<Record<string, number>> {
    if (!this.wallet) return {};

    try {
      // Use NDK's built-in mintBalances getter which uses wallet.state internally
      const allMintBalances = this.wallet.mintBalances || {};

      // Get the mints from the user's kind:10019 event (these are the "authorized" mints)
      const nutzapConfigMints = await this.getNutzapConfigMints();
      const authorizedMints =
        nutzapConfigMints.length > 0
          ? nutzapConfigMints
          : this.wallet.mints || [];

      console.log("🔍 Debug getMintBalances (using NDK methods):");
      console.log("  NDK wallet.mintBalances:", allMintBalances);
      console.log("  Authorized mints:", authorizedMints);

      // Filter to only show balances from authorized mints
      const authorizedBalances: Record<string, number> = {};
      for (const [mint, balance] of Object.entries(allMintBalances)) {
        if (authorizedMints.includes(mint)) {
          authorizedBalances[mint] = balance;
        }
      }

      console.log("  Filtered authorized balances:", authorizedBalances);
      return authorizedBalances;
    } catch (error) {
      console.error("Failed to get mint balances:", error);
      return {};
    }
  }
  
  /**
   * Get mints with sufficient balance using NDK's built-in method
   */
  getMintsWithBalance(amount: number): string[] {
    if (!this.wallet) return [];
    
    try {
      // Use NDK's built-in getMintsWithBalance method
      return this.wallet.getMintsWithBalance(amount);
    } catch (error) {
      console.error("Failed to get mints with balance:", error);
      return [];
    }
  }

  /**
   * Get ALL mint balances using NDK's built-in methods
   * @returns Authorized and unauthorized mint balances
   */
  async getAllMintBalances(): Promise<{
    authorized: Record<string, number>;
    unauthorized: Record<string, number>;
  }> {
    if (!this.wallet) return { authorized: {}, unauthorized: {} };

    try {
      // Use NDK's built-in mintBalances getter
      const allMintBalances = this.wallet.mintBalances || {};

      // Get authorized mints from kind:10019 event
      const nutzapConfigMints = await this.getNutzapConfigMints();
      const authorizedMints =
        nutzapConfigMints.length > 0
          ? nutzapConfigMints
          : this.wallet.mints || [];

      console.log("🔍 Debug getAllMintBalances (using NDK methods):");
      console.log("  NDK wallet.mintBalances:", allMintBalances);
      console.log("  Authorized mints:", authorizedMints);

      const authorized: Record<string, number> = {};
      const unauthorized: Record<string, number> = {};

      for (const [mint, balance] of Object.entries(allMintBalances)) {
        if (authorizedMints.includes(mint)) {
          authorized[mint] = balance;
        } else {
          unauthorized[mint] = balance;
        }
      }

      return { authorized, unauthorized };
    } catch (error) {
      console.error("Failed to get all mint balances:", error);
      return { authorized: {}, unauthorized: {} };
    }
  }

  getCachedBalance(): number {
    return this.cachedBalance;
  }

  // Load cached balance for a specific user
  loadCachedBalanceForUser(userPubkey: string): number {
    const cached = this.storage.getItem(`cashu_balance_${userPubkey}`);
    if (cached) {
      try {
        const { balance, timestamp } = JSON.parse(cached);
        if (Date.now() - timestamp < this.balanceCacheTimeout) {
          return balance;
        }
      } catch (error) {
        console.error("Failed to load cached balance:", error);
      }
    }
    return 0;
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

  // Mint management
  async addMint(mintUrl: string): Promise<void> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
    }

    // Add mint to wallet if not already present
    if (!this.wallet.mints.includes(mintUrl)) {
      this.wallet.mints.push(mintUrl);

      // Update wallet metadata with new mint
      await this.createWalletMetadata(this.wallet.mints);

      // Update kind:10019 to include new mint as authorized
      await this.publishNutzapConfig();

      console.log(`✅ Added mint ${mintUrl} to wallet`);
    }
  }

  async removeMint(mintUrl: string): Promise<void> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
    }

    // Don't allow removing the last mint
    if (this.wallet.mints.length <= 1) {
      throw new Error("Cannot remove the last mint from wallet");
    }

    // Remove mint from wallet
    const mintIndex = this.wallet.mints.indexOf(mintUrl);
    if (mintIndex > -1) {
      this.wallet.mints.splice(mintIndex, 1);

      // Update wallet metadata without the removed mint
      await this.createWalletMetadata(this.wallet.mints);

      // Update kind:10019 to remove mint from authorized list
      await this.publishNutzapConfig();

      // Force balance refresh to exclude removed mint
      await this.updateBalance();

      console.log(`✅ Removed mint ${mintUrl} from wallet`);
    }
  }

  // Transaction history
  getTransactionHistory(): Transaction[] {
    return [...this.transactionHistory];
  }

  addTransaction(transaction: Transaction): void {
    this.transactionHistory = [transaction, ...this.transactionHistory].slice(
      0,
      100
    );
    this.saveTransactionHistory();

    // Create NIP-60 spending history event
    this.createSpendingHistoryEvent(transaction);
  }

  // Token operations - use NDK's built-in methods

  async receiveTokens(token: string): Promise<{ amount: number }> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
    }

    // Use NDK's built-in receiveToken method
    const result = await this.wallet.receiveToken(token, "Received Cashu token");
    const amount = result?.amount || 0;

    if (amount > 0) {
      this.addTransaction({
        id: `receive_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
        type: "receive",
        amount: amount,
        mint: "",
        timestamp: Date.now(),
        status: "completed",
        direction: "in",
      });
    }

    await this.updateBalance();
    return { amount };
  }

  // Nutzap operations - using NDK's built-in NDKZapper with cashuPay callback
  async sendNutzap(
    pubkey: string,
    amount: number,
    mint?: string,
    nutzapRelays?: string[] | null
  ): Promise<void> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
    }

    console.log('💸 Starting nutzap send to profile:', { pubkey, amount, nutzapRelays });

    const user = new NDKUser({ pubkey });
    user.ndk = this.ndk;

    // Use NDKZapper for consistent handling
    // NDKZapper is now imported statically

    // Create NDK instance with nutzap relays if provided
    let zapperNdk = this.ndk;
    let tempNdk: NDK | null = null;

    if (nutzapRelays && nutzapRelays.length > 0) {
      console.log('🔗 Creating temporary NDK for nutzap relays:', nutzapRelays);
      // Create a temporary NDK instance with the recipient's nutzap relays
      tempNdk = new NDK({
        explicitRelayUrls: nutzapRelays,
        signer: this.ndk.signer,
      });
      zapperNdk = tempNdk;

      // Connect to the nutzap relays with timeout
      try {
        console.log('🔌 Connecting to nutzap relays...');
        const connectPromise = zapperNdk.connect();
        const timeoutPromise = new Promise((_, reject) => 
          setTimeout(() => reject(new Error("Nutzap relay connection timeout")), 5000)
        );
        await Promise.race([connectPromise, timeoutPromise]);
        console.log('✅ Connected to nutzap relays for profile nutzap');
      } catch (error) {
        console.warn("Failed to connect to some nutzap relays:", error);
        // Continue anyway - some relays might be connected
      }
    }

    console.log('⚡ Creating NDKZapper...');
    const zapper = new NDKZapper(user, amount, "sat", {
      comment: "",
      ndk: zapperNdk, // Use the NDK with nutzap relays
    });

    try {
      console.log('🔧 Setting up cashuPay callback...');
      // Set the cashuPay callback to use our wallet
      zapper.cashuPay = async (payment: any) => {
        try {
          console.log('💰 Executing cashuPay:', payment);
          const result = await this.wallet!.cashuPay({
            ...payment,
            mints: mint ? [mint] : undefined,
          });

          // Validate the result - NDK returns null on failure
          if (!result || !result.proofs || result.proofs.length === 0) {
            console.error("cashuPay failed in zapper - no valid result");
            throw new Error(
              "Failed to create nutzap: recipient may not have a compatible wallet"
            );
          }

          console.log('✅ cashuPay successful:', result);
          return result;
        } catch (error: any) {
          console.error('❌ cashuPay failed:', error);
          throw error;
        }
      };

      // Execute the zap with timeout
      console.log('⚡ Executing zapper.zap()...');
      const zapPromise = zapper.zap();
      const zapTimeoutPromise = new Promise((_, reject) => 
        setTimeout(() => reject(new Error("Zap execution timeout")), 10000)
      );
      const zapResult = await Promise.race([zapPromise, zapTimeoutPromise]) as Map<any, any>;

      console.log("Profile zap result:", zapResult);

      if (!zapResult || zapResult.size === 0) {
        throw new Error("Failed to send nutzap: zapper returned no result");
      }

      // Check if any of the zap splits succeeded
      let hasValidCashuPayment = false;
      let zapError: Error | null = null;

      for (const [, result] of zapResult) {
        if (result instanceof Error) {
          // Capture the error for better messaging
          zapError = result;
        } else if (
          result &&
          "proofs" in result &&
          result.proofs &&
          result.proofs.length > 0
        ) {
          // Found a valid cashu payment
          hasValidCashuPayment = true;
          break;
        }
      }

      if (!hasValidCashuPayment) {
        if (zapError) {
          console.error("Profile zap failed with error:", zapError.message);
          throw new Error(`Failed to send nutzap: ${zapError.message}`);
        } else {
          console.error(
            "Profile zap completed but no valid cashu payment was created"
          );
          throw new Error(
            "Failed to send nutzap: unable to create valid payment"
          );
        }
      }

      console.log(`Nutzap sent successfully with ${amount} sats`);

      // Update balance only if payment succeeded
      this.updateCachedBalance(Math.max(0, this.cachedBalance - amount));
      await this.updateBalance();
    } catch (error) {
      console.error("Profile nutzap failed:", error);
      throw error;
    } finally {
      // Clean up temporary NDK instance to prevent connection leaks
      if (tempNdk) {
        try {
          Array.from(tempNdk.pool.relays.values()).forEach((relay) => {
            try {
              relay.disconnect();
            } catch (err) {
              // Ignore disconnection errors
            }
          });
          tempNdk.pool.removeAllListeners();
        } catch (err) {
          console.warn("Error cleaning up temporary NDK in sendNutzap:", err);
        }
      }
    }
  }

  // Start nutzap monitoring after wallet is initialized
  private async ensureNutzapMonitor(): Promise<void> {
    if (!this.nutzapMonitor && this.wallet && this.userPubkey) {
      await this.startNutzapMonitor();
    }
  }

  async sendNutzapToEvent(
    eventId: string,
    amount: number,
    mint?: string,
    nutzapRelays?: string[] | null
  ): Promise<void> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
    }

    console.log('💸 Starting nutzap send to event:', { eventId, amount, nutzapRelays });

    // Fetch the event to get the author's pubkey
    const event = await this.ndk.fetchEvent(eventId);
    if (!event) {
      throw new Error("Event not found");
    }

    // Create a user for the event author
    const user = new NDKUser({ pubkey: event.pubkey });
    user.ndk = this.ndk;

    // Use NDKZapper to properly create and publish nutzap
    // NDKZapper is now imported statically

    // Create NDK instance with nutzap relays if provided
    let zapperNdk = this.ndk;
    let tempNdk: NDK | null = null;

    if (nutzapRelays && nutzapRelays.length > 0) {
      console.log('🔗 Creating temporary NDK for nutzap relays:', nutzapRelays);
      // Create a temporary NDK instance with the recipient's nutzap relays
      tempNdk = new NDK({
        explicitRelayUrls: nutzapRelays,
        signer: this.ndk.signer,
      });
      zapperNdk = tempNdk;

      // Connect to the nutzap relays with timeout
      try {
        console.log('🔌 Connecting to nutzap relays for event...');
        const connectPromise = zapperNdk.connect();
        const timeoutPromise = new Promise((_, reject) => 
          setTimeout(() => reject(new Error("Nutzap relay connection timeout")), 5000)
        );
        await Promise.race([connectPromise, timeoutPromise]);
        console.log('✅ Connected to nutzap relays for event nutzap');
      } catch (error) {
        console.warn("Failed to connect to some nutzap relays:", error);
        // Continue anyway - some relays might be connected
      }
    }

    console.log('⚡ Creating NDKZapper for event...');
    const zapper = new NDKZapper(event, amount, "sat", {
      comment: "",
      ndk: zapperNdk, // Use the NDK with nutzap relays
    });

    try {
      console.log('🔧 Setting up cashuPay callback for event...');
      // Set the cashuPay callback to use our wallet
      zapper.cashuPay = async (payment: any) => {
        try {
          console.log('💰 Executing cashuPay for event:', payment);
          const result = await this.wallet!.cashuPay({
            ...payment,
            mints: mint ? [mint] : undefined,
          });

          // Validate the result - NDK returns null on failure
          if (!result || !result.proofs || result.proofs.length === 0) {
            console.error("cashuPay failed in zapper - no valid result");
            throw new Error(
              "Failed to create nutzap: recipient may not have a compatible wallet"
            );
          }

          console.log('✅ cashuPay successful for event:', result);
          return result;
        } catch (error: any) {
          // Ignore swap-related errors as they're non-fatal
          // The wallet will fall back to using existing proofs
          if (
            error.message?.includes("Not enough funds available") &&
            error.message?.includes("swap")
          ) {
            console.warn(
              "Swap optimization failed, continuing with existing proofs"
            );
            // Let the error propagate so NDK can handle the fallback
          }
          console.error('❌ cashuPay failed for event:', error);
          throw error;
        }
      };

      // Execute the zap - this will create and publish the nutzap event
      console.log('⚡ Executing zapper.zap() for event...');
      const zapPromise = zapper.zap();
      const zapTimeoutPromise = new Promise((_, reject) => 
        setTimeout(() => reject(new Error("Event zap execution timeout")), 10000)
      );
      const zapResult = await Promise.race([zapPromise, zapTimeoutPromise]) as Map<any, any>;

      console.log("Zap result:", zapResult);

      if (!zapResult || zapResult.size === 0) {
        throw new Error("Failed to send nutzap: zapper returned no result");
      }

      // Check if any of the zap splits succeeded
      let hasValidCashuPayment = false;
      let zapError: Error | null = null;

      for (const [, result] of zapResult) {
        if (result instanceof Error) {
          // Capture the error for better messaging
          zapError = result;
        } else if (
          result &&
          "proofs" in result &&
          result.proofs &&
          result.proofs.length > 0
        ) {
          // Found a valid cashu payment
          hasValidCashuPayment = true;
          break;
        }
      }

      if (!hasValidCashuPayment) {
        if (zapError) {
          console.error("Zap failed with error:", zapError.message);
          throw new Error(`Failed to send nutzap: ${zapError.message}`);
        } else {
          console.error("Zap completed but no valid cashu payment was created");
          throw new Error(
            "Failed to send nutzap: unable to create valid payment"
          );
        }
      }

      console.log(`Nutzap sent to event successfully with ${amount} sats`);

      // Update balance only if payment succeeded
      this.updateCachedBalance(Math.max(0, this.cachedBalance - amount));
      await this.updateBalance();
    } catch (error) {
      console.error("Nutzap failed:", error);
      throw error;
    } finally {
      // Clean up temporary NDK instance to prevent connection leaks
      if (tempNdk) {
        try {
          Array.from(tempNdk.pool.relays.values()).forEach((relay) => {
            try {
              relay.disconnect();
            } catch (err) {
              // Ignore disconnection errors
            }
          });
          tempNdk.pool.removeAllListeners();
        } catch (err) {
          console.warn(
            "Error cleaning up temporary NDK in sendNutzapToEvent:",
            err
          );
        }
      }
    }
  }

  /**
   * Force refresh wallet proofs and balances
   */
  async refreshWalletState(): Promise<void> {
    if (!this.wallet) return;

    try {
      console.log("🔄 Refreshing wallet state...");

      // The wallet should refresh its state when we access balance
      const balance = await this.getBalance();
      console.log("  Total balance after refresh:", balance);

      // Get mint balances to trigger calculation
      const mintBalances = await this.getMintBalances();
      console.log("  Mint balances after refresh:", mintBalances);
    } catch (error) {
      console.error("Failed to refresh wallet state:", error);
    }
  }

  /**
   * Get mints from the user's kind:10019 event using NDKCashuMintList
   * These are the "authorized" mints according to NIP-61
   */
  private async getNutzapConfigMints(): Promise<string[]> {
    try {
      if (!this.userPubkey) return [];

      // Fetch the user's kind:10019 event
      const filter = {
        kinds: [10019],
        authors: [this.userPubkey],
        limit: 1,
      };

      const events = await this.ndk.fetchEvents(filter);
      if (events.size === 0) return [];

      const event = Array.from(events)[0];
      
      // Use NDKCashuMintList for cleaner parsing
      const mintList = NDKCashuMintList.from(event);
      return mintList.mints; // Returns deduplicated array
      
    } catch (error) {
      console.error("Failed to fetch nutzap config mints:", error);
      return [];
    }
  }

  // Mint operations
  async mintTokens(
    mintUrl: string,
    amount: number
  ): Promise<{ invoice: string; quote: any }> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
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
        if (
          startError.message.includes("400") ||
          startError.message.includes("Bad Request")
        ) {
          throw new Error(
            "This mint rejected the request (400 Bad Request). Try using a different mint like 'https://mint.minibits.cash' or use 'Receive Cashu Token' instead."
          );
        }
        if (
          startError.message.includes("Failed to fetch") ||
          startError.message.includes("CORS")
        ) {
          throw new Error(
            "Cannot connect to mint. This might be due to CORS restrictions when running on localhost. Try using a different mint or running the app on a proper domain."
          );
        }
        if (startError.message.includes("net::ERR_")) {
          throw new Error(
            "Network error connecting to mint. Make sure you have internet connection and the mint is accessible."
          );
        }
      }
      throw startError;
    }

    if (!invoice) {
      throw new Error(
        "Failed to generate invoice. The mint may not support this payment method."
      );
    }

    const quote = {
      id: quoteId || "temp_quote",
      mint: mintUrl,
      deposit: deposit, // Keep reference to the deposit object for monitoring
    };

    this.addTransaction({
      id: `mint_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
      type: "mint",
      amount: amount,
      mint: mintUrl,
      timestamp: Date.now(),
      status: "pending",
    });

    return { invoice, quote };
  }

  async checkAndClaimTokens(
    _mintUrl: string,
    quote: any
  ): Promise<{ proofs: Proof[]; claimed: boolean }> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
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
        console.warn("Payment check failed:", error);
      }

      return { proofs: [], claimed: false };
    }

    // Fallback: wait and check balance
    await new Promise((resolve) => setTimeout(resolve, 2000));
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
        limit: 1,
      };

      const events = await this.ndk.fetchEvents(filter);
      if (events.size === 0) {
        const fallbackRelays = [
          "wss://relay.damus.io",
          "wss://relay.primal.net",
          "wss://relay.nostr.band",
        ];
        return fallbackRelays;
      }

      const relayListEvent = Array.from(events)[0];
      const relays: string[] = [];

      // Parse relay tags
      relayListEvent.tags.forEach((tag) => {
        if (tag[0] === "r" && tag[1]) {
          relays.push(tag[1]);
        }
      });

      // If no relays found in tags, use fallback
      if (relays.length === 0) {
        return [
          "wss://relay.damus.io",
          "wss://relay.primal.net",
          "wss://relay.nostr.band",
        ];
      }

      return relays;
    } catch (error) {
      console.error("Failed to fetch user relays:", error);
      return [
        "wss://relay.damus.io",
        "wss://relay.primal.net",
        "wss://relay.nostr.band",
      ];
    }
  }

  private walletP2PKPrivkey: string | null = null;
  private walletP2PKPubkey: string | null = null;

  private async fetchExistingWallet(
    user: NDKUser
  ): Promise<NDKCashuWallet | null> {
    try {
      // Get user's relays - this is critical for wallet functionality
      const userRelays = await this.getUserRelays(user.pubkey);

      // Create a temporary NDK instance with user's relays
      const userNdk = new NDK({
        explicitRelayUrls: userRelays,
        signer: this.ndk.signer,
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

      // Parse mints from wallet metadata events ONLY - don't auto-add from tokens
      const walletMints = new Set<string>();
      const tokenMints = new Set<string>(); // Track mints from tokens separately
      let tokenCount = 0;

      for (const event of events) {
        if (event.kind === 17375) {
          // Wallet metadata event - extract mints from tags and P2PK private key from content
          const mintTags = event.tags.filter(
            (tag) => tag[0] === "mint" && tag[1]
          );

          // Try to decrypt the wallet metadata to get the P2PK private key
          try {
            await event.decrypt();
            if (event.content) {
              const metadata = JSON.parse(event.content);
              if (metadata.privkey) {
                this.walletP2PKPrivkey = metadata.privkey;
                console.log("🔑 Found P2PK private key in wallet metadata");

                // Get the corresponding public key
                // NDKPrivateKeySigner is now imported statically
                const signer = new NDKPrivateKeySigner(metadata.privkey);
                const pubkey = await signer.user().then((u) => u.pubkey);
                this.walletP2PKPubkey = pubkey;
                console.log("📝 Wallet P2PK pubkey:", pubkey);
              }
            }
          } catch (err) {
            console.warn("⚠️ Could not decrypt wallet metadata:", err);
          }
          // Only add mints from wallet metadata, not from tokens
          mintTags.forEach((tag) => walletMints.add(tag[1]));
        } else if (event.kind === 7375) {
          // Token event
          tokenCount++;

          // First check for mint in tags
          const mintTag = event.tags.find((tag) => tag[0] === "mint" && tag[1]);
          if (mintTag) {
            tokenMints.add(mintTag[1]); // Track token mints separately
          } else {
            // No mint tag, need to decrypt content to find mint
            try {
              // Decrypt the event if needed
              if (
                !event.content.startsWith("{") &&
                !event.content.startsWith("[")
              ) {
                await event.decrypt();
              }

              // Parse decrypted content
              const tokenData = JSON.parse(event.content);

              // Extract mint from token data
              if (tokenData.mint) {
                tokenMints.add(tokenData.mint); // Track token mints separately
              } else if (tokenData.token && Array.isArray(tokenData.token)) {
                // Token format might be nested
                tokenData.token.forEach((t: any) => {
                  if (t.mint) {
                    tokenMints.add(t.mint); // Track token mints separately
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

      console.log(
        `Found ${tokenCount} token events across ${tokenMints.size} mints`
      );
      console.log(`Wallet metadata contains ${walletMints.size} mints`);

      // Only set mints from wallet metadata, NOT from tokens
      // This respects the user's mint preferences
      if (walletMints.size > 0) {
        wallet.mints = Array.from(walletMints);
        console.log("✅ Setting wallet mints from metadata:", wallet.mints);
      } else if (tokenMints.size > 0) {
        // If no wallet metadata but we have tokens, warn the user
        console.warn(
          "⚠️ No mints in wallet metadata but found tokens from:",
          Array.from(tokenMints)
        );
        // Don't auto-add mints - user needs to explicitly accept them
      }

      // The wallet will process the events when we call start()
      await wallet.start();

      // Note: Nutzap monitoring will be started when this wallet is assigned to the service

      // Wait for the wallet to be ready
      await new Promise<void>((resolve) => {
        wallet.once("ready", () => {
          console.log("📦 Wallet ready event received");
          resolve();
        });

        // If wallet is already ready, resolve immediately
        if (
          wallet.status !== NDKWalletStatus.LOADING &&
          wallet.status !== NDKWalletStatus.INITIAL
        ) {
          resolve();
        }
      });

      // Don't auto-update wallet metadata - respect user's mint choices
      // Metadata should only be updated when user explicitly adds/removes mints

      return wallet;
    } catch (error) {
      console.error("Failed to fetch existing wallet:", error);
      return null;
    }
  }

  private async createWalletMetadata(mints: string[]): Promise<void> {
    let tempNdkCleanup: (() => void) | null = null;

    try {
      const user = await this.ndk.signer?.user();
      if (!user) return;

      // Get user's relays - wallet metadata needs to be published everywhere
      const userRelays = await this.getUserRelays(user.pubkey);
      const allRelays = [...new Set([...userRelays])]; // Could add group relay if needed

      // Create temporary NDK with all relays
      const allNdk = new NDK({
        explicitRelayUrls: allRelays,
        signer: this.ndk.signer,
      });

      // Connect with timeout
      try {
        await allNdk.connect();
        // Set up cleanup function
        tempNdkCleanup = () => {
          try {
            Array.from(allNdk.pool.relays.values()).forEach((relay) => {
              try {
                relay.disconnect();
              } catch (err) {
                // Ignore disconnection errors
              }
            });
            allNdk.pool.removeAllListeners();
          } catch (err) {
            console.warn(
              "Error cleaning up temp NDK in createWalletMetadata:",
              err
            );
          }
        };
      } catch (err) {
        console.warn(
          "⚠️ Create wallet metadata: Some relays may not have connected:",
          err
        );
      }

      // Create wallet metadata event
      const walletEvent = new NDKEvent(allNdk);
      walletEvent.kind = 17375; // Replaceable event

      // Wallet metadata structure (as tag arrays for NIP-60)
      walletEvent.tags = mints.map((mint) => ["mint", mint]);

      // Generate a P2PK private key for the wallet
      // NDKPrivateKeySigner is now imported statically
      const walletSigner = NDKPrivateKeySigner.generate();
      const walletPrivkey = walletSigner.privateKey;
      this.walletP2PKPrivkey = walletPrivkey!;
      console.log("🔑 Generated new P2PK private key for wallet");
      const walletPubkey = await walletSigner.user().then((u) => u.pubkey);
      this.walletP2PKPubkey = walletPubkey;

      // Create wallet metadata with the P2PK private key
      const walletMetadata = {
        privkey: walletPrivkey,
      };

      // Encrypt the content using NIP-44
      walletEvent.content = JSON.stringify(walletMetadata);
      const ndkUser = allNdk.getUser({ pubkey: user.pubkey });
      await walletEvent.encrypt(ndkUser, undefined, "nip44");

      await walletEvent.sign();
      await walletEvent.publish();

      console.log(
        "Created/updated NIP-60 wallet metadata to all relays with mints:",
        mints
      );

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
    } finally {
      // Clean up temporary NDK instance
      if (tempNdkCleanup) {
        tempNdkCleanup();
      }
    }
  }

  /**
   * Add P2PK key to existing wallet that doesn't have one
   */
  private async addP2PKKeyToExistingWallet(): Promise<void> {
    let tempNdkCleanup: (() => void) | null = null;

    try {
      const user = await this.ndk.signer?.user();
      if (!user || !this.wallet) return;

      // Generate a P2PK private key for the wallet
      // NDKPrivateKeySigner is now imported statically
      const walletSigner = NDKPrivateKeySigner.generate();
      const walletPrivkey = walletSigner.privateKey;
      this.walletP2PKPrivkey = walletPrivkey!;
      console.log("🔑 Generated new P2PK private key for existing wallet");
      const walletPubkey = await walletSigner.user().then((u) => u.pubkey);
      this.walletP2PKPubkey = walletPubkey;

      // Update the wallet metadata event with the new P2PK key
      const walletMetadata = {
        privkey: walletPrivkey,
      };

      // Get user's relays - wallet metadata needs to be published everywhere
      const userRelays = await this.getUserRelays(user.pubkey);
      const allRelays = [...new Set([...userRelays])];

      // Create temporary NDK with all relays
      const allNdk = new NDK({
        explicitRelayUrls: allRelays,
        signer: this.ndk.signer,
      });

      // Connect with timeout
      try {
        await allNdk.connect();
        // Set up cleanup function
        tempNdkCleanup = () => {
          try {
            Array.from(allNdk.pool.relays.values()).forEach((relay) => {
              try {
                relay.disconnect();
              } catch (err) {
                // Ignore disconnection errors
              }
            });
            allNdk.pool.removeAllListeners();
          } catch (err) {
            console.warn(
              "Error cleaning up temp NDK in addP2PKKeyToExistingWallet:",
              err
            );
          }
        };
      } catch (err) {
        console.warn(
          "⚠️ Update wallet metadata: Some relays may not have connected:",
          err
        );
      }

      // Create wallet metadata event
      const walletEvent = new NDKEvent(allNdk);
      walletEvent.kind = 17375; // Replaceable event

      // Wallet metadata structure (as tag arrays for NIP-60)
      walletEvent.tags = this.wallet.mints.map((mint) => ["mint", mint]);

      // Encrypt the content using NIP-44
      walletEvent.content = JSON.stringify(walletMetadata);
      const ndkUser = allNdk.getUser({ pubkey: user.pubkey });
      await walletEvent.encrypt(ndkUser, undefined, "nip44");

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
        console.log("🔄 Restarting nutzap monitor with new P2PK key...");
        this.nutzapMonitor.stop();
        this.nutzapMonitor = null;
        await this.startNutzapMonitor();
      }
    } catch (error) {
      console.error("❌ Failed to add P2PK key to existing wallet:", error);
    } finally {
      // Clean up temporary NDK instance
      if (tempNdkCleanup) {
        tempNdkCleanup();
      }
    }
  }

  /**
   * Publish kind 10019 event to enable nutzap receiving using NDKCashuMintList
   * This should be called whenever mints are added/removed
   */
  async publishNutzapConfig(): Promise<void> {
    if (!this.walletP2PKPubkey) {
      console.warn("⚠️ Cannot publish nutzap config: no P2PK pubkey");
      return;
    }

    try {
      const user = await this.ndk.signer?.user();
      if (!user) return;

      // Get current relays from NDK
      const relays = Array.from(this.ndk.pool.relays.values()).map(
        (r) => r.url
      );

      // For testing, ensure we include the local relay
      if (!relays.includes("ws://localhost:8080")) {
        relays.push("ws://localhost:8080");
      }

      // Get mints from wallet - these become the "authorized" mints
      const mints = this.wallet?.mints || [];

      // Create kind 10019 event using NDKCashuMintList
      const mintList = new NDKCashuMintList(this.ndk);

      // Set properties using NDKCashuMintList's clean API
      mintList.relays = relays;
      mintList.mints = mints;
      mintList.p2pk = this.walletP2PKPubkey;

      // Convert to event and publish
      await mintList.toNostrEvent();
      await mintList.publishReplaceable();

      console.log("✅ Published kind 10019 nutzap config event using NDKCashuMintList");
      console.log("📝 Nutzap config:", {
        relays,
        mints,
        p2pk: this.walletP2PKPubkey,
      });
    } catch (error) {
      console.error("❌ Failed to publish nutzap config:", error);
    }
  }

  private async updateBalance(): Promise<void> {
    const balance = await this.getBalance();
    this.notifyBalanceCallbacks(balance);
  }

  private async startNutzapMonitor(): Promise<void> {
    if (!this.wallet || !this.userPubkey || this.nutzapMonitor) {
      console.warn(
        "🚨 Cannot start nutzap monitor: wallet not initialized or monitor already running"
      );
      return;
    }

    try {
      // Create nutzap monitor
      const user = await this.ndk.signer?.user();
      if (!user) {
        console.error("🔴 Cannot create nutzap monitor: no user");
        return;
      }
      // Create monitor with relay configuration
      // Get the current relays from NDK pool
      const relays = Array.from(this.ndk.pool.relays.values());

      // Create a relay set that includes our local relay
      // NDKRelaySet is now imported statically
      const relaySet = new NDKRelaySet(new Set(relays), this.ndk);

      // Create monitor with custom options
      // Pass the relay set through a custom property that the monitor can use
      const monitorOptions: any = {
        relaySet: relaySet,
      };
      this.nutzapMonitor = new NDKNutzapMonitor(this.ndk, user, monitorOptions);

      // Set the wallet for redemption
      (this.nutzapMonitor as any).wallet = this.wallet;

      // Add the WALLET's P2PK private key for nutzap redemption
      // According to NIP-60, nutzaps use a separate wallet private key, NOT the user's Nostr key
      // This key is extracted from the kind 17375 wallet metadata event
      try {
        if (this.walletP2PKPrivkey) {
          // NDKPrivateKeySigner is now imported statically
          const privkeySigner = new NDKPrivateKeySigner(this.walletP2PKPrivkey);
          this.nutzapMonitor.addPrivkey(privkeySigner);
          console.log("🔑 Added wallet P2PK private key to nutzap monitor");
        } else {
          console.warn(
            "⚠️ Wallet does not have a P2PK private key - cannot redeem nutzaps"
          );
          // The NDKCashuWallet doesn't expose the privkey, and we couldn't extract it from the metadata event
          console.warn(
            "⚠️ This means the wallet was likely created without a P2PK private key"
          );
        }
      } catch (error) {
        console.error("🔴 Error setting up wallet private key:", error);
      }

      // Listen for all nutzap monitor events
      this.nutzapMonitor.on("seen", () => {});

      this.nutzapMonitor.on("seen_in_unknown_mint", () => {});

      this.nutzapMonitor.on("state_changed", () => {});

      // Listen for redeemed nutzaps
      this.nutzapMonitor.on("redeemed", async (event: any) => {
        console.log("🎉 Nutzap redeemed!", event);
        // Update balance after redemption
        await this.updateBalance();

        // Extract amount from the event
        const amount = event.amount || event.nutzap?.amount || 0;

        // Add to transaction history
        this.addTransaction({
          id: event.id || `nutzap_${Date.now()}`,
          type: "receive",
          amount: amount,
          timestamp: Date.now(),
          status: "completed",
          description: "Received nutzap",
          mint: event.mint || "unknown",
        });
      });

      // Listen for errors
      this.nutzapMonitor.on("failed", (event: any) => {
        console.error("🔴 Nutzap redemption failed:", event);
      });

      // Log subscription status

      // Set the relay set on the monitor if possible
      if ("relaySet" in this.nutzapMonitor) {
        (this.nutzapMonitor as any).relaySet = monitorOptions.relaySet;
      }

      // Set the wallet on the monitor for redemption
      if ("wallet" in this.nutzapMonitor) {
        (this.nutzapMonitor as any).wallet = this.wallet;
      }

      // Start monitoring with a filter for nutzap events
      const filter = { kinds: [9321], "#p": [this.userPubkey!] };

      // Start the monitor - it may throw backup key errors but will still work
      this.nutzapMonitor
        .start({
          filter,
          opts: {
            closeOnEose: false,
            groupable: false,
          },
        })
        .catch((error: any) => {
          // Log but don't fail - backup keys are optional
          console.warn("⚠️ Nutzap monitor backup key warning:", error.message);
        });

      console.log("✅ Nutzap monitor started successfully");
    } catch (error) {
      console.error("🔴 Failed to start nutzap monitor:", error);
      // Don't throw - the wallet can still function without nutzap monitoring
    }
  }

  /**
   * Set up wallet event listeners to leverage NDK's event handling
   */
  private setupWalletEventListeners(): void {
    if (!this.wallet) return;

    // Listen for wallet events using NDK's EventEmitter capabilities
    this.wallet.on('ready', () => {
      console.log('🎉 Wallet is ready');
      this.updateBalance();
    });

    this.wallet.on('balance_updated', () => {
      console.log('💰 Wallet balance updated by NDK');
      this.updateBalance();
    });

    this.wallet.on('warning', (warning: any) => {
      console.warn('⚠️ Wallet warning:', warning);
    });

    // Listen for deposit events
    this.wallet.depositMonitor?.on('change', () => {
      console.log('🔄 Deposit monitor change detected');
      this.updateBalance();
    });

    console.log('✅ Set up wallet event listeners');
  }

  private updateCachedBalance(balance: number): void {
    this.cachedBalance = balance;
    this.persistBalanceCache(balance);
    this.notifyBalanceCallbacks(balance);
  }

  private notifyBalanceCallbacks(balance: number): void {
    this.balanceCallbacks.forEach((callback) => callback(balance));
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
      timestamp: Date.now(),
    });
    this.storage.setItem(`cashu_balance_${this.userPubkey}`, data);
  }

  private loadTransactionHistoryFromStorage(): void {
    const stored = this.storage.getItem("cashu_transactions");
    if (stored) {
      try {
        this.transactionHistory = JSON.parse(stored);
      } catch (error) {
        console.error("Failed to load transaction history:", error);
      }
    }
  }

  private saveTransactionHistory(): void {
    this.storage.setItem(
      "cashu_transactions",
      JSON.stringify(this.transactionHistory)
    );
  }

  private async createSpendingHistoryEvent(
    transaction: Transaction
  ): Promise<void> {
    let tempNdkCleanup: (() => void) | null = null;

    try {
      if (!this.userPubkey) return;

      // Get user's relays for publishing
      const userRelays = await this.getUserRelays(this.userPubkey);
      const allRelays = [...new Set([...userRelays])];

      // Create temporary NDK with all relays
      const allNdk = new NDK({
        explicitRelayUrls: allRelays,
        signer: this.ndk.signer,
      });

      // Connect with timeout
      try {
        await allNdk.connect();
        // Set up cleanup function
        tempNdkCleanup = () => {
          try {
            Array.from(allNdk.pool.relays.values()).forEach((relay) => {
              try {
                relay.disconnect();
              } catch (err) {
                // Ignore disconnection errors
              }
            });
            allNdk.pool.removeAllListeners();
          } catch (err) {
            console.warn(
              "Error cleaning up temp NDK in createSpendingHistoryEvent:",
              err
            );
          }
        };
      } catch (err) {
        console.warn(
          "⚠️ Create spending history: Some relays may not have connected:",
          err
        );
      }

      const historyEvent = new NDKEvent(allNdk);
      historyEvent.kind = 7376;

      // Transaction data
      const txData = {
        direction:
          transaction.direction ||
          (transaction.type === "receive" ? "in" : "out"),
        amount: transaction.amount,
        unit: "sat",
        mint: transaction.mint,
        description: transaction.description || transaction.type,
      };

      // Encrypt content with NIP-44
      historyEvent.content = JSON.stringify(txData);

      await historyEvent.sign();
      await historyEvent.publish();

      console.log("📝 Created NIP-60 spending history event");
    } catch (error) {
      console.error("Failed to create spending history event:", error);
    } finally {
      // Clean up temporary NDK instance
      if (tempNdkCleanup) {
        tempNdkCleanup();
      }
    }
  }

  private async loadTransactionHistory(): Promise<void> {
    let tempNdkCleanup: (() => void) | null = null;

    try {
      if (!this.userPubkey) return;

      const userRelays = await this.getUserRelays(this.userPubkey);
      const userNdk = new NDK({
        explicitRelayUrls: userRelays,
        signer: this.ndk.signer,
      });

      // Connect with timeout
      try {
        const connectPromise = userNdk.connect();
        const timeoutPromise = new Promise((_, reject) =>
          setTimeout(() => reject(new Error("Connection timeout")), 3000)
        );
        await Promise.race([connectPromise, timeoutPromise]);
        // Set up cleanup function
        tempNdkCleanup = () => {
          try {
            Array.from(userNdk.pool.relays.values()).forEach((relay) => {
              try {
                relay.disconnect();
              } catch (err) {
                // Ignore disconnection errors
              }
            });
            userNdk.pool.removeAllListeners();
          } catch (err) {
            console.warn(
              "Error cleaning up temp NDK in loadTransactionHistory:",
              err
            );
          }
        };
      } catch (err) {
        console.debug("Some relays may not have connected:", err);
      }

      // Fetch spending history events (kind 7376)
      const filter = {
        kinds: [7376],
        authors: [this.userPubkey],
        limit: 100,
      };

      const events = await userNdk.fetchEvents(filter);

      if (events.size === 0) {
        return;
      }

      // Process history events
      const existingIds = new Set(this.transactionHistory.map((tx) => tx.id));
      let addedCount = 0;

      for (const event of events) {
        try {
          let content;

          // Check if content is encrypted
          if (event.content.startsWith("{")) {
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
              if (tag[0] === "direction" && tag[1]) {
                transactionData.direction = tag[1];
              } else if (tag[0] === "amount" && tag[1]) {
                transactionData.amount = parseInt(tag[1]);
              } else if (tag[0] === "mint" && tag[1]) {
                transactionData.mint = tag[1];
              } else if (tag[0] === "description" && tag[1]) {
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
            type:
              transactionData.direction === "in"
                ? "receive"
                : transactionData.direction === "out"
                  ? "send"
                  : "mint",
            amount: transactionData.amount || 0,
            mint: transactionData.mint || "Unknown",
            timestamp: (event.created_at || 0) * 1000, // Convert to milliseconds
            status: "completed",
            direction: transactionData.direction,
            description: transactionData.description,
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
    } finally {
      // Clean up temporary NDK instance
      if (tempNdkCleanup) {
        tempNdkCleanup();
      }
    }
  }

  /**
   * Melt Cashu tokens to pay a Lightning invoice
   * This converts ecash tokens back to Lightning
   * @param invoice Lightning invoice to pay
   * @returns Payment confirmation with preimage if successful
   */
  async meltToLightning(invoice: string): Promise<{
    paid: boolean;
    preimage?: string;
    fee?: number;
    error?: string;
  }> {
    if (!this.wallet) {
      throw new Error("Wallet not initialized");
    }

    try {
      console.log("🔥 Using NDK's built-in lnPay method...");

      // Use NDK's built-in lnPay method which handles melt quote creation and proof selection
      const paymentResult = await this.wallet.lnPay({ pr: invoice });

      if (paymentResult && paymentResult.preimage) {
        // Extract amount from invoice
        const invoiceAmount = this.parseInvoiceAmount(invoice);

        // Add transaction record
        this.addTransaction({
          id: `melt_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`,
          type: "melt",
          amount: invoiceAmount,
          mint: this.wallet.mints[0],
          timestamp: Date.now(),
          status: "completed",
          description: `Lightning payment: ${invoice.substring(0, 20)}...`,
        });

        // Update balance
        await this.updateBalance();

        return {
          paid: true,
          preimage: paymentResult.preimage,
          fee: 0, // NDK doesn't expose fee details
        };
      }

      return {
        paid: false,
        error: "Payment failed - no preimage received",
      };
    } catch (error) {
      console.error("❌ Lightning payment failed:", error);
      return {
        paid: false,
        error: error instanceof Error ? error.message : "Unknown error",
      };
    }
  }

  /**
   * Parse amount from Lightning invoice
   * Basic implementation - in production use a proper bolt11 decoder
   */
  private parseInvoiceAmount(invoice: string): number {
    try {
      // Lightning invoices have the amount encoded in them
      // Format: lnbc<amount><multiplier>...
      const match = invoice.match(/lnbc(\d+)([munp]?)/i);
      if (!match) {
        return 0;
      }

      const amount = parseInt(match[1]);
      const multiplier = match[2] || "";

      // Convert to sats based on multiplier
      switch (multiplier) {
        case "m":
          return amount * 100000; // millisats to sats
        case "u":
          return amount * 100; // microsats to sats
        case "n":
          return amount / 10; // nanosats to sats
        case "p":
          return amount / 10000; // picosats to sats
        default:
          return amount; // already in sats
      }
    } catch (error) {
      console.error("Failed to parse invoice amount:", error);
      return 0;
    }
  }

  // ========================================
  // CASHU EVENT PARSING METHODS
  // ========================================

  /**
   * Parse nutzap relays from kind:10019 event using NDKCashuMintList
   */
  parseNutzapRelays(event10019: any): string[] {
    try {
      if (!event10019) return [];
      
      // Use NDKCashuMintList for cleaner relay parsing
      const mintList = NDKCashuMintList.from(event10019);
      const relays = mintList.relays || [];
      
      // Basic filtering for nutzap relays
      return this.filterNutzapRelays(relays);
    } catch (error) {
      console.error("Failed to parse nutzap relays using NDKCashuMintList:", error);
      
      // Fallback to manual parsing if NDKCashuMintList fails
      const relays: string[] = [];
      if (event10019?.tags) {
        event10019.tags.forEach((tag: string[]) => {
          if (tag[0] === 'relay' && tag[1]) {
            relays.push(tag[1]);
          }
        });
      }
      return this.filterNutzapRelays(relays);
    }
  }

  /**
   * Parse nutzap mints from kind:10019 event using NDKCashuMintList
   */
  parseNutzapMints(event10019: any): string[] {
    try {
      if (!event10019) return [];
      
      // Use NDKCashuMintList for cleaner mint parsing
      const mintList = NDKCashuMintList.from(event10019);
      return mintList.mints || [];
    } catch (error) {
      console.error("Failed to parse nutzap mints using NDKCashuMintList:", error);
      
      // Fallback to manual parsing if NDKCashuMintList fails
      const mints: string[] = [];
      if (event10019?.tags) {
        event10019.tags.forEach((tag: string[]) => {
          if (tag[0] === 'mint' && tag[1]) {
            mints.push(tag[1]);
          }
        });
      }
      return mints;
    }
  }

  /**
   * Get P2PK from kind:10019 event using NDKCashuMintList
   */
  parseNutzapP2PK(event10019: any): string | null {
    try {
      if (!event10019) return null;
      
      // Use NDKCashuMintList for cleaner P2PK parsing
      const mintList = NDKCashuMintList.from(event10019);
      return mintList.p2pk || null;
    } catch (error) {
      console.error("Failed to parse nutzap P2PK using NDKCashuMintList:", error);
      
      // Fallback to manual parsing if NDKCashuMintList fails
      if (event10019?.tags) {
        const pubkeyTag = event10019.tags.find((tag: string[]) => tag[0] === 'pubkey' && tag[1]);
        if (pubkeyTag) {
          return pubkeyTag[1];
        }
      }
      // Fall back to event pubkey if no explicit pubkey tag
      return event10019?.pubkey || null;
    }
  }

  /**
   * Fetch user's kind:10019 event with caching
   */
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
          // Fetch kind:10019 event
          const filter = {
            kinds: [10019],
            authors: [pubkey],
            limit: 1
          };

          // Fetch with timeout
          const fetchPromise = this.ndk.fetchEvents(filter);
          const timeoutPromise = new Promise((_, reject) => 
            setTimeout(() => reject(new Error("User 10019 fetch timeout")), 8000)
          );
          
          const events = await Promise.race([fetchPromise, timeoutPromise]) as Set<any>;
          
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

  /**
   * Fetch multiple users' kind:10019 events efficiently
   */
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
      // Fetch all kind:10019 events in one query
      const filter = {
        kinds: [10019],
        authors: pubkeysToFetch,
        limit: pubkeysToFetch.length
      };

      // Fetch events with timeout
      const fetchPromise = this.ndk.fetchEvents(filter);
      const timeoutPromise = new Promise((_, reject) => 
        setTimeout(() => reject(new Error("Bulk user 10019 fetch timeout")), 8000)
      );
      
      const events = await Promise.race([fetchPromise, timeoutPromise]) as Set<any>;
      
      // Map events back to authors and cache them
      events.forEach((event: any) => {
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

  /**
   * Filter nutzap relays with basic validation
   */
  private filterNutzapRelays(relays: string[]): string[] {
    return relays
      .filter(relay => {
        try {
          const url = new URL(relay);
          return url.protocol === 'ws:' || url.protocol === 'wss:';
        } catch {
          return false;
        }
      })
      .slice(0, 5); // Allow up to 5 relays for nutzaps
  }

  // Cleanup method
  dispose(): void {
    // Stop nutzap monitor
    if (this.nutzapMonitor) {
      this.nutzapMonitor.stop();
      this.nutzapMonitor = null;
    }

    this.balanceCallbacks.clear();
    this.user10019Cache.clear();
    this.user10019FetchPromises.clear();
    this.wallet = null;
  }
}
