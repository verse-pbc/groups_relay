import { useState, useEffect } from "preact/hooks";
import type { NostrClient } from "../api/nostr_client";

interface WalletDisplayProps {
  client: NostrClient;
}

export const WalletDisplay = ({ client }: WalletDisplayProps) => {
  const [balance, setBalance] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);

  const initializeWallet = async () => {
    setLoading(true);
    setError(null);
    try {
      // Initialize with some default Cashu mints
      await client.initializeWallet([
        "https://mint.minibits.cash/Bitcoin",
        "https://legend.lnbits.com/cashu/api/v1/4gr9Xcmz3XEkUNwiBiQGoC"
      ]);
      setIsInitialized(true);
      await fetchBalance();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to initialize wallet");
    } finally {
      setLoading(false);
    }
  };

  const fetchBalance = async () => {
    if (!client.walletInstance) return;
    
    setLoading(true);
    try {
      const walletBalance = await client.getWalletBalance();
      setBalance(walletBalance);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to fetch balance");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (client.walletInstance) {
      setIsInitialized(true);
      fetchBalance();
    }
  }, [client]);

  if (!isInitialized) {
    return (
      <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
        <h3 class="text-lg font-semibold mb-3">Cashu Wallet</h3>
        <p class="text-gray-400 mb-3">Initialize your Cashu wallet to enable payments</p>
        <button
          onClick={initializeWallet}
          disabled={loading}
          class="bg-purple-600 hover:bg-purple-700 disabled:bg-gray-600 px-4 py-2 rounded-md text-sm font-medium transition-colors"
        >
          {loading ? "Initializing..." : "Initialize Wallet"}
        </button>
        {error && <p class="text-red-400 text-sm mt-2">{error}</p>}
      </div>
    );
  }

  return (
    <div class="bg-gray-800 rounded-lg p-4 border border-gray-700">
      <div class="flex items-center justify-between mb-3">
        <h3 class="text-lg font-semibold">Cashu Wallet</h3>
        <button
          onClick={fetchBalance}
          disabled={loading}
          class="text-purple-400 hover:text-purple-300 text-sm"
        >
          {loading ? "Refreshing..." : "Refresh"}
        </button>
      </div>
      
      <div class="space-y-2">
        <div class="flex items-center justify-between">
          <span class="text-gray-400">Balance:</span>
          <span class="text-xl font-mono">
            {balance !== null ? `${balance} sats` : "---"}
          </span>
        </div>
        
        {error && <p class="text-red-400 text-sm">{error}</p>}
      </div>

      <div class="mt-4 pt-4 border-t border-gray-700">
        <p class="text-xs text-gray-500">
          Connected to {client.walletInstance?.mints?.length || 0} mint(s)
        </p>
      </div>
    </div>
  );
};