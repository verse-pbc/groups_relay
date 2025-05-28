import { useState, useEffect } from "preact/hooks";
import type { NostrClient, Transaction } from "../api/nostr_client";
import QRCode from "qrcode";

interface WalletDisplayProps {
  client: NostrClient;
  onClose?: () => void;
  isModal?: boolean;
  initialCashuBalance?: number;
}

export const WalletDisplay = ({ client, onClose, isModal, initialCashuBalance }: WalletDisplayProps) => {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [hasCheckedWallet, setHasCheckedWallet] = useState(false);
  const [cashuBalance, setCashuBalance] = useState<number>(initialCashuBalance ?? 0);
  const [mints, setMints] = useState<string[]>([
    "https://mint.minibits.cash/Bitcoin",
    "https://mint.coinos.io"
  ]);
  const [showMintManager, setShowMintManager] = useState(false);
  const [newMintUrl, setNewMintUrl] = useState("");
  const [showReceiveModal, setShowReceiveModal] = useState(false);
  const [receiveMode, setReceiveMode] = useState<'paste' | 'mint'>('paste');
  const [tokenInput, setTokenInput] = useState("");
  const [mintAmount, setMintAmount] = useState("");
  const [mintInvoice, setMintInvoice] = useState("");
  const [isMinting, setIsMinting] = useState(false);
  const [selectedMint, setSelectedMint] = useState("");
  const [qrCodeDataUrl, setQrCodeDataUrl] = useState<string | null>(null);
  const [currentQuote, setCurrentQuote] = useState<any>(null);
  const [showTransactions, setShowTransactions] = useState(false);
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [mintBalances, setMintBalances] = useState<Record<string, number>>({});
  const [checkingPayment, setCheckingPayment] = useState(false);

  const initializeWallet = async () => {
    setLoading(true);
    setError(null);
    setSuccess(null);
    
    try {
      // Show initialized quickly, continue loading in background
      setIsInitialized(true);
      
      // Initialize NDK wallet (will check for existing NIP-60 wallet)
      await client.initializeWallet(mints);
      
      // Get mints from wallet
      const walletMints = await client.getCashuMints();
      if (walletMints.length > 0) {
        setMints(walletMints);
        setSuccess("Wallet initialized successfully!");
      }
      
      // Only fetch balance if not provided as props
      if (initialCashuBalance === undefined) {
        await fetchBalance();
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to initialize wallet");
      setIsInitialized(false);
    } finally {
      setLoading(false);
    }
  };

  const fetchBalance = async (forceRefresh: boolean = false) => {
    if (!client.walletInstance) return;
    
    // Skip if we have initial balance and not forcing refresh
    if (!forceRefresh && initialCashuBalance !== undefined) {
      return;
    }
    
    setLoading(true);
    try {
      // Run all balance fetching operations in parallel
      const [cashuBalance, txHistory] = await Promise.all([
        // Fetch total Cashu balance
        client.getCashuBalance().catch(() => {
          return 0;
        }),
        
        // Load transaction history
        Promise.resolve(client.getTransactionHistory())
      ]);
      
      // Update balance
      setCashuBalance(cashuBalance);
      
      // For now, we don't have per-mint balances with NDKCashuWallet
      // Could be added later if needed
      setMintBalances({});
      
      // Update transaction history
      setTransactions(txHistory.slice(0, 10)); // Show last 10 transactions
      
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to fetch balance");
    } finally {
      setLoading(false);
    }
  };

  const receiveToken = () => {
    setShowReceiveModal(true);
    setError(null);
    setSuccess(null);
    // Auto-select first mint if only one available
    if (mints.length === 1) {
      setSelectedMint(mints[0]);
    }
  };

  const handlePasteToken = async () => {
    if (!tokenInput.trim()) {
      setError("Please paste a Cashu token");
      return;
    }
    
    setLoading(true);
    setError(null);
    try {
      const { amount } = await client.receiveTokens(tokenInput);
      setSuccess(`Received ${amount} sats!`);
      setShowReceiveModal(false);
      setTokenInput("");
      // Refresh balance after receiving tokens
      await fetchBalance();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to receive token");
    } finally {
      setLoading(false);
    }
  };

  const handleRequestInvoice = async () => {
    const amount = parseInt(mintAmount);
    if (!amount || amount <= 0) {
      setError("Please enter a valid amount");
      return;
    }

    if (!mints.length) {
      setError("Please add a mint first");
      return;
    }

    const mintUrl = selectedMint || mints[0];
    if (!mintUrl) {
      setError("Please select a mint");
      return;
    }

    setIsMinting(true);
    setError(null);
    try {
      const { invoice, quote } = await client.mintTokens(mintUrl, amount);
      setMintInvoice(invoice);
      setCurrentQuote(quote);
      
      // Generate QR code for the invoice
      if (invoice) {
        try {
          const qrDataUrl = await QRCode.toDataURL(invoice, {
            width: 256,
            margin: 2,
            color: {
              dark: '#000000',
              light: '#FFFFFF'
            }
          });
          setQrCodeDataUrl(qrDataUrl);
        } catch (qrErr) {
          setError("Failed to generate QR code for invoice");
        }
      } else {
      }
      
      // For testnut mints, check immediately if tokens are available
      if (mintUrl.includes('testnut')) {
        setSuccess("Invoice generated! For testnut, tokens may be available immediately. Click 'Check & Claim Tokens'.");
      } else {
        setSuccess("Invoice generated! Pay it to receive Cashu tokens.");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to generate invoice");
    } finally {
      setIsMinting(false);
    }
  };

  const addMint = async () => {
    if (!newMintUrl.trim()) return;
    
    setLoading(true);
    setError(null);
    try {
      // Use the client's addMint method which handles NIP-60 persistence
      await client.addMint(newMintUrl);
      
      setMints([...mints, newMintUrl]);
      setNewMintUrl("");
      setSuccess("Mint added and saved to wallet!");
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to add mint");
    } finally {
      setLoading(false);
    }
  };

  const removeMint = async (mintUrl: string) => {
    try {
      const updatedMints = mints.filter(m => m !== mintUrl);
      
      // TODO: Add mint management to wallet service
      setMints(updatedMints);
      setSuccess("Mint removed from wallet!");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to remove mint");
    }
  };

  useEffect(() => {
    // Auto-initialize wallet on component mount
    const autoInitialize = async () => {
      if (!isInitialized && !loading && !hasCheckedWallet) {
        setHasCheckedWallet(true);
        await initializeWallet();
      }
    };
    
    autoInitialize();
  }, [client, isInitialized, loading, hasCheckedWallet]);

  useEffect(() => {
    // Clean expired wallet keys periodically
    const interval = setInterval(async () => {
      await client.clearExpiredWalletKeys();
    }, 60 * 60 * 1000); // Every hour
    
    return () => clearInterval(interval);
  }, [client]);
  
  useEffect(() => {
    // Subscribe to balance updates
    const unsubscribe = client.onBalanceUpdate((balance) => {
      setCashuBalance(balance);
    });
    
    return () => unsubscribe();
  }, [client]);

  useEffect(() => {
    if (success) {
      const timer = setTimeout(() => setSuccess(null), 3000);
      return () => clearTimeout(timer);
    }
  }, [success]);

  // Handle ESC key to close modal
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && showReceiveModal) {
        setShowReceiveModal(false);
        setTokenInput("");
        setMintAmount("");
        setMintInvoice("");
        setSelectedMint("");
        setQrCodeDataUrl(null);
        setCurrentQuote(null);
        setError(null);
        setSuccess(null);
      }
    };

    if (showReceiveModal) {
      document.addEventListener('keydown', handleKeyDown);
      return () => document.removeEventListener('keydown', handleKeyDown);
    }
  }, [showReceiveModal]);

  // Auto-check payment status when we have an active invoice
  useEffect(() => {
    if (!mintInvoice || !currentQuote || !selectedMint || checkingPayment) return;

    const checkInterval = setInterval(async () => {
      try {
        setCheckingPayment(true);
        const { claimed } = await client.checkAndClaimTokens(selectedMint, currentQuote);
        
        if (claimed) {
          setSuccess("Payment received! Tokens claimed successfully.");
          setShowReceiveModal(false);
          setMintInvoice("");
          setMintAmount("");
          setQrCodeDataUrl(null);
          setCurrentQuote(null);
          setSelectedMint("");
          await fetchBalance();
          clearInterval(checkInterval);
        }
      } catch (err) {
      } finally {
        setCheckingPayment(false);
      }
    }, 5000); // Check every 5 seconds

    return () => clearInterval(checkInterval);
  }, [mintInvoice, currentQuote, selectedMint]);

  // Auto-initialize on mount for modal view
  useEffect(() => {
    if (isModal && !isInitialized && !loading && !hasCheckedWallet) {
      setHasCheckedWallet(true);
      initializeWallet();
    }
  }, [isModal]);

  const containerClass = isModal 
    ? "bg-[var(--color-bg-secondary)] rounded-lg p-6"
    : "bg-gray-800 rounded-lg p-4 border border-gray-700";

  if (!isInitialized) {
    return (
      <div class={containerClass}>
        <div class="flex items-center justify-between mb-3">
          <h3 class="text-lg font-semibold">Wallet</h3>
          {isModal && onClose && (
            <button
              onClick={onClose}
              class="text-gray-400 hover:text-white transition-colors"
            >
              <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          )}
        </div>
        {loading ? (
          <div class="text-center py-8">
            <div class="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-purple-500"></div>
            <p class="text-gray-400 mt-2">Initializing wallet...</p>
          </div>
        ) : (
          <>
            <p class="text-gray-400 mb-3">Initialize your wallet to enable Ecash payments</p>
            <button
              onClick={initializeWallet}
              disabled={loading}
              class="bg-purple-600 hover:bg-purple-700 disabled:bg-gray-600 px-4 py-2 rounded-md text-sm font-medium transition-colors"
            >
              Initialize Wallet
            </button>
          </>
        )}
        {error && <p class="text-red-400 text-sm mt-2">{error}</p>}
      </div>
    );
  }

  return (
    <>
      <div class={containerClass}>
        <div class="flex items-center justify-between mb-3">
          <h3 class="text-lg font-semibold">Wallet</h3>
          <div class="flex items-center gap-2">
            <button
              onClick={() => fetchBalance(true)}
              disabled={loading}
              class="text-purple-400 hover:text-purple-300 text-sm"
            >
              {loading ? "Refreshing..." : "Refresh"}
            </button>
            {isModal && onClose && (
              <button
                onClick={onClose}
                class="text-gray-400 hover:text-white transition-colors ml-2"
              >
                <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            )}
          </div>
        </div>
      
      <div class="space-y-3">
        <div>
          <div class="flex items-center justify-between">
            <span class="text-gray-400 text-sm">Balance:</span>
            <span class="text-xl font-semibold text-[#f7931a] flex items-center gap-1">
              <span class="text-base">₿</span>
              {cashuBalance.toLocaleString()}
              <span class="text-sm font-normal">sats</span>
            </span>
          </div>
        </div>
        
        {error && <p class="text-sm mt-2 text-red-400">{error}</p>}
        {success && <p class="text-sm mt-2 text-green-400">{success}</p>}
      </div>

      <div class="mt-4 pt-4 border-t border-gray-700 space-y-2">
        <button
          onClick={receiveToken}
          disabled={loading}
          class="w-full bg-[#f7931a] hover:bg-[#f68e0a] disabled:bg-gray-600 px-4 py-3 rounded-lg text-white font-medium transition-all transform hover:scale-[1.02] active:scale-[0.98]"
        >
          Receive Cashu Token
        </button>
        
        <button
          onClick={() => setShowMintManager(!showMintManager)}
          class="w-full bg-[var(--color-bg-tertiary)] hover:bg-gray-700 px-4 py-3 rounded-lg text-sm font-medium transition-all border border-[var(--color-border)]"
        >
          {showMintManager ? 'Hide' : 'Manage'} Mints ({mints.length})
        </button>
        
        <button
          onClick={async () => {
            if (!showTransactions) {
              // Fetch fresh transaction history when showing
              setLoading(true);
              try {
                const txHistory = await client.getTransactionHistory();
                setTransactions(txHistory.slice(0, 10));
              } catch (err) {
              } finally {
                setLoading(false);
              }
            }
            setShowTransactions(!showTransactions);
          }}
          class="w-full bg-[var(--color-bg-tertiary)] hover:bg-gray-700 px-4 py-3 rounded-lg text-sm font-medium transition-all border border-[var(--color-border)]"
        >
          {showTransactions ? 'Hide' : 'Show'} Transaction History
        </button>
        
        {showMintManager && (
          <div class="space-y-2 pt-2 border-t border-gray-700">
            <div class="text-xs text-gray-400">Connected Mints:</div>
            {mints.map(mint => {
              const balance = mintBalances[mint] || 0;
              return (
                <div key={mint} class="flex items-center justify-between text-xs">
                  <span class="text-gray-300 truncate flex-1 mr-2">
                    {new URL(mint).hostname}
                    {balance > 0 && (
                      <span class="text-[#f7931a] ml-2">₿{balance} sats</span>
                    )}
                  </span>
                  <button
                    onClick={() => removeMint(mint)}
                    class="text-red-400 hover:text-red-300"
                  >
                    Remove
                  </button>
                </div>
              );
            })}
            
            <div class="flex gap-2 mt-2">
              <input
                type="text"
                value={newMintUrl}
                onInput={(e) => setNewMintUrl((e.target as HTMLInputElement).value)}
                placeholder="https://mint.example.com"
                class="flex-1 px-2 py-1 bg-gray-700 border border-gray-600 rounded text-xs"
              />
              <button
                onClick={addMint}
                disabled={loading || !newMintUrl}
                class="px-3 py-1 bg-purple-600 hover:bg-purple-700 disabled:bg-gray-600 rounded text-xs"
              >
                Add
              </button>
            </div>
          </div>
        )}
        
        {showTransactions && (
          <div class="space-y-2 pt-2 border-t border-gray-700">
            <div class="text-xs text-gray-400">Recent Transactions:</div>
            {transactions.length === 0 ? (
              <p class="text-xs text-gray-500">No transactions yet</p>
            ) : (
              <div class="space-y-1 max-h-48 overflow-y-auto">
                {transactions.map(tx => (
                  <div key={tx.id} class="flex items-center justify-between text-xs py-2 px-3 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)] hover:border-[var(--color-border-hover)] transition-colors">
                    <div class="flex items-center gap-2">
                      <span class={`font-medium flex items-center gap-1 ${
                        tx.type === 'receive' ? 'text-green-400' : 
                        tx.type === 'send' ? 'text-red-400' : 
                        tx.type === 'mint' ? 'text-purple-400' : 
                        'text-gray-400'
                      }`}>
                        {tx.type === 'receive' ? '+' : 
                         tx.type === 'send' ? '-' : 
                         tx.type === 'mint' ? '⚡' : ''}
                        <span class="text-[#f7931a]">₿</span>
                        <span>{tx.amount.toLocaleString()}</span>
                        <span class="text-xs font-normal">sats</span>
                      </span>
                      <span class="text-gray-500">
                        {tx.type.charAt(0).toUpperCase() + tx.type.slice(1)}
                      </span>
                    </div>
                    <div class="flex items-center gap-2">
                      <span class={`px-1 py-0.5 rounded text-xs ${
                        tx.status === 'completed' ? 'bg-green-900/30 text-green-400' : 
                        tx.status === 'pending' ? 'bg-yellow-900/30 text-yellow-400' : 
                        'bg-red-900/30 text-red-400'
                      }`}>
                        {tx.status}
                      </span>
                      <span class="text-gray-500">
                        {new Date(tx.timestamp).toLocaleTimeString()}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
    
    {/* Receive Token Modal */}
    {showReceiveModal && (
      <div class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50 p-4">
        <div class="bg-gray-800 rounded-lg p-6 max-w-md w-full max-h-[90vh] overflow-y-auto">
          <h3 class="text-lg font-semibold mb-4">Receive Cashu Tokens</h3>
          
          {/* Mode selector */}
          <div class="flex mb-4 bg-gray-700 rounded-lg p-1">
            <button
              onClick={() => {
                setReceiveMode('paste');
                setError(null);
                setSuccess(null);
              }}
              class={`flex-1 py-2 px-3 rounded-md text-sm font-medium transition-colors ${
                receiveMode === 'paste' 
                  ? 'bg-gray-800 text-white' 
                  : 'text-gray-400 hover:text-white'
              }`}
            >
              Paste Token
            </button>
            <button
              onClick={() => {
                setReceiveMode('mint');
                setError(null);
                setSuccess(null);
              }}
              class={`flex-1 py-2 px-3 rounded-md text-sm font-medium transition-colors ${
                receiveMode === 'mint' 
                  ? 'bg-gray-800 text-white' 
                  : 'text-gray-400 hover:text-white'
              }`}
            >
              Mint New
            </button>
          </div>

          {/* Paste Token Mode */}
          {receiveMode === 'paste' && (
            <div class="space-y-4">
              <div>
                <label class="block text-sm font-medium text-gray-400 mb-2">
                  Paste your Cashu token
                </label>
                <textarea
                  value={tokenInput}
                  onInput={(e) => setTokenInput((e.target as HTMLTextAreaElement).value)}
                  placeholder="cashuAeyJ0..."
                  class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-sm
                         placeholder-gray-500 focus:outline-none focus:ring-1 focus:ring-purple-500
                         font-mono"
                  rows={4}
                />
              </div>
              
              <button
                onClick={handlePasteToken}
                disabled={loading || !tokenInput.trim()}
                class="w-full bg-[#f7931a] hover:bg-[#f68e0a] disabled:bg-gray-600 
                       px-4 py-3 rounded-lg text-sm font-medium transition-all
                       transform hover:scale-[1.02] active:scale-[0.98] disabled:transform-none"
              >
                {loading ? 'Receiving...' : 'Receive Token'}
              </button>
            </div>
          )}

          {/* Mint New Mode */}
          {receiveMode === 'mint' && (
            <div class="space-y-4">
              {!mintInvoice ? (
                <>
                  <div>
                    <label class="block text-sm font-medium text-gray-400 mb-2">
                      Select Mint
                    </label>
                    <select
                      value={selectedMint}
                      onChange={(e) => setSelectedMint((e.target as HTMLSelectElement).value)}
                      class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded-lg text-sm
                             focus:outline-none focus:ring-2 focus:ring-accent/20 focus:border-accent transition-all"
                    >
                      <option value="">Select a mint...</option>
                      {mints.map(mint => (
                        <option key={mint} value={mint}>
                          {new URL(mint).hostname}
                        </option>
                      ))}
                    </select>
                    {window.location.hostname === 'localhost' && (
                      <p class="text-xs text-yellow-400 mt-1">
                        ⚠️ Running on localhost may cause CORS issues with some mints
                      </p>
                    )}
                  </div>
                  
                  <div>
                    <label class="block text-sm font-medium text-gray-400 mb-2">
                      Amount (sats)
                    </label>
                    <input
                      type="number"
                      value={mintAmount}
                      onInput={(e) => setMintAmount((e.target as HTMLInputElement).value)}
                      placeholder="1000"
                      min="1"
                      class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded-lg text-sm
                             placeholder-[var(--color-text-tertiary)] focus:outline-none focus:ring-2 focus:ring-accent/20 focus:border-accent transition-all"
                    />
                  </div>
                  
                  <button
                    onClick={handleRequestInvoice}
                    disabled={isMinting || !mintAmount || parseInt(mintAmount) <= 0}
                    class="w-full bg-purple-600 hover:bg-purple-700 disabled:bg-gray-600 
                           px-4 py-3 rounded-lg text-sm font-medium transition-all
                           transform hover:scale-[1.02] active:scale-[0.98] disabled:transform-none"
                  >
                    {isMinting ? 'Generating...' : 'Generate Invoice'}
                  </button>
                </>
              ) : (
                <div class="space-y-4">
                  {/* QR Code */}
                  {qrCodeDataUrl && (
                    <div class="flex justify-center">
                      <img 
                        src={qrCodeDataUrl} 
                        alt="Invoice QR Code" 
                        class="rounded-lg"
                      />
                    </div>
                  )}
                  
                  <div>
                    <label class="block text-sm font-medium text-gray-400 mb-2">
                      Invoice
                    </label>
                    <div class="p-3 bg-[var(--color-bg-primary)] rounded-lg break-all border border-[var(--color-border)]">
                      <code class="text-xs text-green-400 font-mono">{mintInvoice}</code>
                    </div>
                  </div>
                  
                  <div class="flex items-center gap-2 text-xs text-gray-400">
                    <span class="flex items-center gap-1">
                      Amount: <span class="text-[#f7931a]">₿{mintAmount} sats</span>
                    </span>
                    <span>•</span>
                    <span>Mint: {new URL(selectedMint || mints[0]).hostname}</span>
                  </div>
                  
                  <div class="space-y-2">
                    <button
                      onClick={() => navigator.clipboard.writeText(mintInvoice)}
                      class="w-full bg-purple-600 hover:bg-purple-700 px-4 py-3 rounded-lg text-sm font-medium transition-all
                             transform hover:scale-[1.02] active:scale-[0.98]"
                    >
                      Copy Invoice
                    </button>
                    
                    <button
                      onClick={() => {
                        setMintInvoice("");
                        setMintAmount("");
                        setQrCodeDataUrl(null);
                        setCurrentQuote(null);
                      }}
                      class="w-full bg-[var(--color-bg-tertiary)] hover:bg-gray-700 px-4 py-3 rounded-lg text-sm font-medium transition-all border border-[var(--color-border)]"
                    >
                      Generate New Invoice
                    </button>
                  </div>
                  
                  {checkingPayment && (
                    <div class="bg-purple-900/20 border border-purple-700/30 rounded-lg p-3">
                      <div class="text-xs text-purple-300 flex items-center gap-2">
                        <div class="inline-block animate-spin rounded-full h-3 w-3 border-b-2 border-purple-300"></div>
                        Checking payment status...
                      </div>
                    </div>
                  )}
                  
                  <button
                    onClick={async () => {
                      if (!currentQuote || !selectedMint) {
                        setError("No active quote found");
                        return;
                      }
                      
                      setLoading(true);
                      setError(null);
                      try {
                        const mintUrl = selectedMint || mints[0];
                        const { claimed } = await client.checkAndClaimTokens(mintUrl, currentQuote);
                        
                        if (claimed) {
                          setSuccess(`Successfully claimed tokens!`);
                          setShowReceiveModal(false);
                          setMintInvoice("");
                          setMintAmount("");
                          setQrCodeDataUrl(null);
                          setCurrentQuote(null);
                          setSelectedMint("");
                          
                          // Force refresh balance after claiming tokens
                          await fetchBalance(true);
                        } else {
                          setError("Invoice not paid yet. For testnut mints, try again in a moment.");
                        }
                      } catch (err) {
                        setError(err instanceof Error ? err.message : "Failed to check/claim tokens");
                      } finally {
                        setLoading(false);
                      }
                    }}
                    disabled={loading || !currentQuote || checkingPayment}
                    class="w-full bg-[#f7931a] hover:bg-[#f68e0a] disabled:bg-gray-600 
                           px-4 py-3 rounded-lg text-sm font-medium transition-all
                           transform hover:scale-[1.02] active:scale-[0.98] disabled:transform-none"
                  >
                    {loading || checkingPayment ? 'Checking...' : 'Check & Claim Tokens Manually'}
                  </button>
                  
                  <p class="text-xs text-gray-400 text-center">
                    For testnut mints, tokens may be issued automatically without payment
                  </p>
                </div>
              )}
            </div>
          )}

          {/* Error/Success Messages */}
          {error && <p class="text-sm mt-4 text-red-400">{error}</p>}
          {success && <p class="text-sm mt-4 text-green-400">{success}</p>}

          {/* Close button */}
          <button
            onClick={() => {
              setShowReceiveModal(false);
              setTokenInput("");
              setMintAmount("");
              setMintInvoice("");
              setSelectedMint("");
              setQrCodeDataUrl(null);
              setCurrentQuote(null);
              setError(null);
              setSuccess(null);
            }}
            class="mt-4 w-full bg-[var(--color-bg-tertiary)] hover:bg-gray-700 px-4 py-3 rounded-lg text-sm font-medium transition-all border border-[var(--color-border)]"
          >
            Close
          </button>
        </div>
      </div>
    )}
    </>
  );
};