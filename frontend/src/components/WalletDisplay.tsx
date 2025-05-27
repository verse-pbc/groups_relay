import { useState, useEffect } from "preact/hooks";
import type { NostrClient, Transaction } from "../api/nostr_client";
import QRCode from "qrcode";

interface WalletDisplayProps {
  client: NostrClient;
  onClose?: () => void;
  isModal?: boolean;
}

export const WalletDisplay = ({ client, onClose, isModal }: WalletDisplayProps) => {
  const [balance, setBalance] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);
  const [hasCheckedWallet, setHasCheckedWallet] = useState(false);
  const [cashuBalance, setCashuBalance] = useState<number>(0);
  const [mints, setMints] = useState<string[]>([
    "https://testnut.cashu.space",
    "https://nofees.testnut.cashu.space",
    "https://mint.minibits.cash/Bitcoin"
  ]);
  const [showMintManager, setShowMintManager] = useState(false);
  const [newMintUrl, setNewMintUrl] = useState("");
  const [showReceiveModal, setShowReceiveModal] = useState(false);
  const [receiveMode, setReceiveMode] = useState<'paste' | 'mint'>('paste');
  const [tokenInput, setTokenInput] = useState("");
  const [mintAmount, setMintAmount] = useState("");
  const [lightningInvoice, setLightningInvoice] = useState("");
  const [isMinting, setIsMinting] = useState(false);
  const [selectedMint, setSelectedMint] = useState("");
  const [qrCodeDataUrl, setQrCodeDataUrl] = useState<string | null>(null);
  const [currentQuote, setCurrentQuote] = useState<any>(null);
  const [showTransactions, setShowTransactions] = useState(false);
  const [transactions, setTransactions] = useState<Transaction[]>([]);

  const initializeWallet = async () => {
    setLoading(true);
    setError(null);
    setSuccess(null);
    
    try {
      // Show initialized quickly, continue loading in background
      setIsInitialized(true);
      
      // Initialize NDK wallet (will check for existing NIP-60 wallet)
      await client.initializeWallet(mints);
      
      // Check if we got mints from NIP-60
      if (client.walletInstance?.mints && client.walletInstance.mints.length > 0) {
        // Use mints from NIP-60 wallet
        const nip60Mints = client.walletInstance.mints;
        
        // Filter out default mints that NDK might have added
        const uniqueMints = Array.from(new Set(nip60Mints));
        const hasNonDefaultMints = uniqueMints.some(mint => 
          !mints.includes(mint)
        );
        
        if (hasNonDefaultMints || uniqueMints.length > 0) {
          setMints(uniqueMints);
          setSuccess("Restored existing wallet from your relays!");
          console.log("🎉 Wallet UI: Using mints from NIP-60:", uniqueMints);
          
          // Initialize Cashu mints from NIP-60 in parallel
          await Promise.all(uniqueMints.map(async (mint) => {
            try {
              await client.initializeCashuMint(mint);
            } catch (mintErr) {
              console.warn(`⚠️ Failed to initialize mint ${mint}:`, mintErr);
            }
          }));
        } else {
          console.log("📝 Wallet UI: NIP-60 wallet found but using default mints");
          // Initialize default Cashu mints in parallel
          await Promise.all(mints.map(async (mint) => {
            try {
              await client.initializeCashuMint(mint);
            } catch (mintErr) {
              console.warn(`⚠️ Failed to initialize mint ${mint}:`, mintErr);
            }
          }));
        }
      } else {
        console.log("📝 Wallet UI: No NIP-60 wallet found, using default mints:", mints);
        // Initialize default Cashu mints in parallel
        await Promise.all(mints.map(async (mint) => {
          try {
            await client.initializeCashuMint(mint);
          } catch (mintErr) {
            console.warn(`⚠️ Failed to initialize mint ${mint}:`, mintErr);
          }
        }));
      }
      
      // Fetch balance after initialization
      await fetchBalance();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to initialize wallet");
      setIsInitialized(false);
    } finally {
      setLoading(false);
    }
  };

  const fetchBalance = async () => {
    if (!client.walletInstance) return;
    
    setLoading(true);
    try {
      // Prune spent proofs first
      await client.pruneAllSpentProofs();
      
      // Fetch NDK wallet balance
      const walletBalance = await client.getWalletBalance();
      setBalance(walletBalance);
      
      // Fetch Cashu balance from all mints
      let totalCashuBalance = 0;
      for (const mintUrl of mints) {
        try {
          const mintBalance = await client.getCashuBalance(mintUrl);
          totalCashuBalance += mintBalance;
          console.log(`💰 Balance for ${mintUrl}: ${mintBalance} sats`);
        } catch (err) {
          console.warn(`Failed to get balance from ${mintUrl}:`, err);
        }
      }
      setCashuBalance(totalCashuBalance);
      
      // Load transaction history
      const txHistory = client.getTransactionHistory();
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
      setLightningInvoice(invoice);
      setCurrentQuote(quote);
      
      // Generate QR code for the invoice
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
        console.error("Failed to generate QR code:", qrErr);
      }
      
      // For testnut mints, check immediately if tokens are available
      if (mintUrl.includes('testnut')) {
        setSuccess("Invoice generated! For testnut, tokens may be available immediately. Click 'Check & Claim Tokens'.");
      } else {
        setSuccess("Lightning invoice generated! Pay it to receive Cashu tokens.");
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
      await client.initializeCashuMint(newMintUrl);
      setMints([...mints, newMintUrl]);
      setNewMintUrl("");
      setSuccess("Mint added successfully!");
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to add mint");
    } finally {
      setLoading(false);
    }
  };

  const removeMint = (mintUrl: string) => {
    setMints(mints.filter(m => m !== mintUrl));
  };

  useEffect(() => {
    // Auto-initialize wallet on component mount
    const autoInitialize = async () => {
      if (!isInitialized && !loading && !hasCheckedWallet) {
        console.log("🚀 Auto-initializing wallet...");
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
    if (success) {
      const timer = setTimeout(() => setSuccess(null), 3000);
      return () => clearTimeout(timer);
    }
  }, [success]);

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
            <p class="text-gray-400 mb-3">Initialize your wallet to enable Lightning and Ecash payments</p>
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
              onClick={fetchBalance}
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
            <span class="text-gray-400 text-sm">Lightning Balance:</span>
            <span class="text-lg font-mono">
              {balance !== null ? `${balance} sats` : "---"}
            </span>
          </div>
        </div>

        <div>
          <div class="flex items-center justify-between">
            <span class="text-gray-400 text-sm">Cashu Balance:</span>
            <span class="text-lg font-mono text-green-400">
              {cashuBalance} sats
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
          class="w-full bg-green-600 hover:bg-green-700 disabled:bg-gray-600 px-3 py-2 rounded-md text-sm font-medium transition-colors"
        >
          Receive Cashu Token
        </button>
        
        <button
          onClick={() => setShowMintManager(!showMintManager)}
          class="w-full bg-gray-600 hover:bg-gray-700 px-3 py-2 rounded-md text-sm font-medium transition-colors"
        >
          {showMintManager ? 'Hide' : 'Manage'} Mints ({mints.length})
        </button>
        
        <button
          onClick={() => setShowTransactions(!showTransactions)}
          class="w-full bg-gray-600 hover:bg-gray-700 px-3 py-2 rounded-md text-sm font-medium transition-colors"
        >
          {showTransactions ? 'Hide' : 'Show'} Transaction History
        </button>
        
        {showMintManager && (
          <div class="space-y-2 pt-2 border-t border-gray-700">
            <div class="text-xs text-gray-400">Connected Mints:</div>
            {mints.map(mint => (
              <div key={mint} class="flex items-center justify-between text-xs">
                <span class="text-gray-300 truncate flex-1 mr-2">
                  {new URL(mint).hostname}
                </span>
                <button
                  onClick={() => removeMint(mint)}
                  class="text-red-400 hover:text-red-300"
                >
                  Remove
                </button>
              </div>
            ))}
            
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
                  <div key={tx.id} class="flex items-center justify-between text-xs py-1 px-2 bg-gray-700 rounded">
                    <div class="flex items-center gap-2">
                      <span class={`font-medium ${
                        tx.type === 'receive' ? 'text-green-400' : 
                        tx.type === 'send' ? 'text-red-400' : 
                        tx.type === 'mint' ? 'text-blue-400' : 
                        'text-gray-400'
                      }`}>
                        {tx.type === 'receive' ? '+' : 
                         tx.type === 'send' ? '-' : 
                         tx.type === 'mint' ? '⚡' : ''}
                        {tx.amount} sats
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
                class="w-full bg-green-600 hover:bg-green-700 disabled:bg-gray-600 
                       px-4 py-2 rounded-lg text-sm font-medium transition-colors"
              >
                {loading ? 'Receiving...' : 'Receive Token'}
              </button>
            </div>
          )}

          {/* Mint New Mode */}
          {receiveMode === 'mint' && (
            <div class="space-y-4">
              {!lightningInvoice ? (
                <>
                  {mints.length > 1 && (
                    <div>
                      <label class="block text-sm font-medium text-gray-400 mb-2">
                        Select Mint
                      </label>
                      <select
                        value={selectedMint}
                        onChange={(e) => setSelectedMint((e.target as HTMLSelectElement).value)}
                        class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-sm
                               focus:outline-none focus:ring-1 focus:ring-purple-500"
                      >
                        <option value="">Select a mint...</option>
                        {mints.map(mint => (
                          <option key={mint} value={mint}>
                            {new URL(mint).hostname}
                          </option>
                        ))}
                      </select>
                    </div>
                  )}
                  
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
                      class="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-lg text-sm
                             placeholder-gray-500 focus:outline-none focus:ring-1 focus:ring-purple-500"
                    />
                  </div>
                  
                  <button
                    onClick={handleRequestInvoice}
                    disabled={isMinting || !mintAmount || parseInt(mintAmount) <= 0}
                    class="w-full bg-orange-600 hover:bg-orange-700 disabled:bg-gray-600 
                           px-4 py-2 rounded-lg text-sm font-medium transition-colors"
                  >
                    {isMinting ? 'Generating...' : 'Generate Lightning Invoice'}
                  </button>
                </>
              ) : (
                <div class="space-y-4">
                  {/* QR Code */}
                  {qrCodeDataUrl && (
                    <div class="flex justify-center">
                      <img 
                        src={qrCodeDataUrl} 
                        alt="Lightning Invoice QR Code" 
                        class="rounded-lg"
                      />
                    </div>
                  )}
                  
                  <div>
                    <label class="block text-sm font-medium text-gray-400 mb-2">
                      Lightning Invoice
                    </label>
                    <div class="p-3 bg-gray-900 rounded-lg break-all">
                      <code class="text-xs text-green-400">{lightningInvoice}</code>
                    </div>
                  </div>
                  
                  <div class="flex items-center gap-2 text-xs text-gray-400">
                    <span>Amount: {mintAmount} sats</span>
                    <span>•</span>
                    <span>Mint: {new URL(selectedMint || mints[0]).hostname}</span>
                  </div>
                  
                  <div class="space-y-2">
                    <button
                      onClick={() => navigator.clipboard.writeText(lightningInvoice)}
                      class="w-full bg-purple-600 hover:bg-purple-700 px-4 py-2 rounded-lg text-sm font-medium transition-colors"
                    >
                      Copy Invoice
                    </button>
                    
                    <button
                      onClick={() => {
                        setLightningInvoice("");
                        setMintAmount("");
                        setQrCodeDataUrl(null);
                        setCurrentQuote(null);
                      }}
                      class="w-full bg-gray-600 hover:bg-gray-700 px-4 py-2 rounded-lg text-sm font-medium transition-colors"
                    >
                      Generate New Invoice
                    </button>
                  </div>
                  
                  <div class="bg-blue-900/20 border border-blue-700/30 rounded-lg p-3">
                    <p class="text-xs text-blue-300">
                      <strong>Testnut Info:</strong>
                    </p>
                    <ul class="text-xs text-blue-300 mt-1 space-y-1 list-disc list-inside">
                      <li>Testnut mints issue fake tokens for testing</li>
                      <li>Invoices may be auto-paid (no real payment needed)</li>
                      <li>Check mint status after generating invoice</li>
                    </ul>
                  </div>
                  
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
                        const { proofs, claimed } = await client.checkAndClaimTokens(mintUrl, currentQuote);
                        
                        if (claimed) {
                          setSuccess(`Successfully claimed ${proofs.length} tokens! Total: ${proofs.reduce((sum, p) => sum + p.amount, 0)} sats`);
                          setShowReceiveModal(false);
                          setLightningInvoice("");
                          setMintAmount("");
                          setQrCodeDataUrl(null);
                          setCurrentQuote(null);
                          setSelectedMint("");
                          
                          // Refresh balance
                          await fetchBalance();
                        } else {
                          setError("Invoice not paid yet. For testnut mints, try again in a moment.");
                        }
                      } catch (err) {
                        setError(err instanceof Error ? err.message : "Failed to check/claim tokens");
                      } finally {
                        setLoading(false);
                      }
                    }}
                    disabled={loading || !currentQuote}
                    class="w-full bg-green-600 hover:bg-green-700 disabled:bg-gray-600 
                           px-4 py-2 rounded-lg text-sm font-medium transition-colors"
                  >
                    {loading ? 'Checking...' : 'Check & Claim Tokens'}
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
              setLightningInvoice("");
              setSelectedMint("");
              setQrCodeDataUrl(null);
              setCurrentQuote(null);
              setError(null);
              setSuccess(null);
            }}
            class="mt-4 w-full bg-gray-700 hover:bg-gray-600 px-4 py-2 rounded-lg text-sm font-medium transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    )}
    </>
  );
};