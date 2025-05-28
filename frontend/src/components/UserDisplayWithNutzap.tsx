import { Component } from 'preact'
import { UserDisplay } from './UserDisplay'
import { NostrClient } from '../api/nostr_client'
import type { Proof } from '@cashu/cashu-ts'

interface UserDisplayWithNutzapProps {
  pubkey: string
  client: NostrClient
  cashuProofs?: Proof[]
  mints?: string[]
  onSendNutzap?: () => void
  showCopy?: boolean
  size?: 'sm' | 'md' | 'lg'
  isRelayAdmin?: boolean
  onCopy?: () => void
  hideNutzap?: boolean
}

interface UserDisplayWithNutzapState {
  showNutzapModal: boolean
  sending: boolean
  amount: string
  comment: string
  error: string | null
  walletBalance: number
  selectedMint: string
  mintBalances: Record<string, number>
}

export class UserDisplayWithNutzap extends Component<UserDisplayWithNutzapProps, UserDisplayWithNutzapState> {
  state = {
    showNutzapModal: false,
    sending: false,
    amount: '',
    comment: '',
    error: null,
    walletBalance: 0,
    selectedMint: '',
    mintBalances: {}
  }

  componentDidMount() {
    // Add ESC key listener
    document.addEventListener('keydown', this.handleKeyDown);
  }

  componentWillUnmount() {
    // Remove ESC key listener
    document.removeEventListener('keydown', this.handleKeyDown);
  }

  handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && this.state.showNutzapModal) {
      this.setState({ 
        showNutzapModal: false,
        amount: '',
        comment: '',
        error: null
      });
    }
  }

  // Fetch wallet balance and mint balances when modal opens
  fetchWalletBalance = async () => {
    try {
      const { mints } = this.props
      const mintBalances: Record<string, number> = {}
      
      // Get total balance (per-mint balance not supported yet in NDK)
      const totalBalance = await this.props.client.getCashuBalance()
      this.setState({ walletBalance: totalBalance })
      
      if (mints && mints.length > 0) {
        // For now, we can't get per-mint balances, so we'll just show the total
        // balance for display purposes in the dropdown
        for (const mint of mints) {
          // Show total balance next to each mint until per-mint is supported
          mintBalances[mint] = totalBalance
        }
        
        this.setState({ 
          mintBalances,
          selectedMint: mints[0] || ''
        })
      }
    } catch (error) {
      console.error('Failed to fetch wallet balance:', error)
      this.setState({ walletBalance: 0, mintBalances: {} })
    }
  }

  handleSendNutzap = async () => {
    const { client, pubkey, onSendNutzap } = this.props
    const { amount } = this.state
    
    const sats = parseInt(amount)
    if (!sats || sats <= 0) {
      this.setState({ error: 'Please enter a valid amount' })
      return
    }

    // Check against total balance
    const totalBalance = this.state.walletBalance
    if (sats > totalBalance) {
      this.setState({ error: `Insufficient balance (${totalBalance} sats available)` })
      return
    }

    this.setState({ sending: true, error: null })
    
    try {
      // Convert npub to hex if needed
      const hexPubkey = pubkey.startsWith('npub') ? client.npubToPubkey(pubkey) : pubkey
      
      // Let NDK choose the best mint
      await client.sendNutzap(hexPubkey, sats)
      
      // SUCCESS - Update balance optimistically (without re-fetching from mints)
      const currentBalance = this.state.walletBalance
      const newBalance = Math.max(0, currentBalance - sats)
      
      // Update balance and notify other components
      this.setState({ 
        showNutzapModal: false, 
        amount: '', 
        comment: '',
        error: null,
        walletBalance: newBalance
      })
      
      client.notifyBalanceUpdate(newBalance)
      
      if (onSendNutzap) onSendNutzap()
    } catch (error) {
      // Error - just show the error, no balance changes needed
      this.setState({ 
        error: error instanceof Error ? error.message : 'Failed to send nutzap' 
      })
    } finally {
      this.setState({ sending: false })
    }
  }

  render() {
    const { pubkey, client, showCopy, size, isRelayAdmin, onCopy, hideNutzap } = this.props
    const { showNutzapModal, sending, amount, comment, error } = this.state
    
    // Check if wallet has balance (NDKCashuWallet doesn't expose proofs directly)
    const hasWalletBalance = client.hasWalletBalance()

    return (
      <div class="flex items-center gap-2">
        <UserDisplay 
          pubkey={pubkey}
          client={client}
          showCopy={showCopy}
          size={size}
          isRelayAdmin={isRelayAdmin}
          onCopy={onCopy}
        />
        
        {!hideNutzap && hasWalletBalance && (
          <button
            onClick={() => {
              this.setState({ showNutzapModal: true })
              this.fetchWalletBalance()
            }}
            class="shrink-0 p-1.5 text-[#f7931a] hover:text-[#f68e0a] bg-[#f7931a]/10 hover:bg-[#f7931a]/20 rounded transition-colors"
            title="Send nutzap"
          >
            <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </button>
        )}

        {showNutzapModal && (
          <>
            {/* Modal backdrop */}
            <div 
              class="fixed inset-0 bg-black/50 z-50" 
              onClick={() => this.setState({ showNutzapModal: false })}
            />
            
            {/* Modal */}
            <div class="fixed top-1/2 left-1/2 transform -translate-x-1/2 -translate-y-1/2 bg-[var(--color-bg-secondary)] rounded-xl border border-[var(--color-border)] p-6 z-50 w-96 max-w-[90vw] shadow-xl">
              <h3 class="text-lg font-semibold mb-4">Send Nutzap</h3>
              
              <div class="space-y-4">
                <div>
                  <label class="block text-sm text-[var(--color-text-secondary)] mb-1">
                    Available Mints
                  </label>
                  <div class="text-xs text-[var(--color-text-tertiary)] space-y-1">
                    {Object.entries(this.state.mintBalances).map(([mint]) => (
                      <div key={mint}>• {new URL(mint).hostname}</div>
                    ))}
                  </div>
                  <p class="text-xs text-[var(--color-text-tertiary)] mt-2">
                    Note: The wallet will automatically select the best mint for this transaction.
                  </p>
                </div>

                <div>
                  <label class="block text-sm text-[var(--color-text-secondary)] mb-1">
                    Amount
                  </label>
                  <input
                    type="number"
                    value={amount}
                    onInput={(e) => this.setState({ amount: (e.target as HTMLInputElement).value })}
                    placeholder="64"
                    class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded-lg text-[var(--color-text-primary)] focus:outline-none focus:ring-2 focus:ring-accent/20 focus:border-accent transition-all"
                    disabled={sending}
                  />
                  <p class="text-xs text-[var(--color-text-tertiary)] mt-1 flex items-center gap-1">
                    Total balance: <span class="text-[#f7931a] font-medium">₿{this.state.walletBalance.toLocaleString()} sats</span>
                  </p>
                </div>

                <div>
                  <label class="block text-sm text-[var(--color-text-secondary)] mb-1">
                    Comment (optional)
                  </label>
                  <textarea
                    value={comment}
                    onInput={(e) => this.setState({ comment: (e.target as HTMLTextAreaElement).value })}
                    placeholder="Thanks for the help!"
                    rows={3}
                    class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded-lg text-[var(--color-text-primary)] resize-none focus:outline-none focus:ring-2 focus:ring-accent/20 focus:border-accent transition-all"
                    disabled={sending}
                  />
                </div>

                {error && (
                  <p class="text-red-400 text-sm">{error}</p>
                )}

                <div class="flex gap-2">
                  <button
                    onClick={this.handleSendNutzap}
                    disabled={sending || !amount}
                    class="flex-1 bg-[#f7931a] hover:bg-[#f68e0a] disabled:bg-gray-600 px-4 py-3 rounded-lg text-white font-medium transition-all transform hover:scale-[1.02] active:scale-[0.98] disabled:transform-none disabled:cursor-not-allowed"
                  >
                    {sending ? 'Sending...' : 'Send Nutzap'}
                  </button>
                  <button
                    onClick={() => this.setState({ showNutzapModal: false })}
                    disabled={sending}
                    class="px-4 py-3 bg-[var(--color-bg-primary)] hover:bg-[var(--color-bg-tertiary)] rounded-lg text-sm font-medium transition-all border border-[var(--color-border)]"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            </div>
          </>
        )}
      </div>
    )
  }
}