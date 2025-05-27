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
}

export class UserDisplayWithNutzap extends Component<UserDisplayWithNutzapProps, UserDisplayWithNutzapState> {
  state = {
    showNutzapModal: false,
    sending: false,
    amount: '',
    comment: '',
    error: null
  }

  handleSendNutzap = async () => {
    const { client, pubkey, cashuProofs, mints, onSendNutzap } = this.props
    const { amount, comment } = this.state
    
    const sats = parseInt(amount)
    if (!sats || sats <= 0) {
      this.setState({ error: 'Please enter a valid amount' })
      return
    }

    const totalBalance = cashuProofs ? cashuProofs.reduce((sum, proof) => sum + proof.amount, 0) : 0
    if (sats > totalBalance) {
      this.setState({ error: 'Insufficient balance' })
      return
    }

    if (!mints || mints.length === 0) {
      this.setState({ error: 'No mints available' })
      return
    }

    this.setState({ sending: true, error: null })
    
    try {
      // Convert npub to hex if needed
      const hexPubkey = pubkey.startsWith('npub') ? client.npubToPubkey(pubkey) : pubkey
      
      // For now, use the first mint. In the future, implement multi-mint payments
      const selectedMint = mints[0]
      
      await client.sendNutzap(hexPubkey, sats, cashuProofs || [], selectedMint, comment)
      
      this.setState({ 
        showNutzapModal: false, 
        amount: '', 
        comment: '',
        error: null 
      })
      
      if (onSendNutzap) onSendNutzap()
    } catch (error) {
      this.setState({ 
        error: error instanceof Error ? error.message : 'Failed to send nutzap' 
      })
    } finally {
      this.setState({ sending: false })
    }
  }

  render() {
    const { pubkey, client, showCopy, size, isRelayAdmin, cashuProofs, onCopy, hideNutzap } = this.props
    const { showNutzapModal, sending, amount, comment, error } = this.state
    
    const totalBalance = cashuProofs ? cashuProofs.reduce((sum, proof) => sum + proof.amount, 0) : 0

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
        
        {!hideNutzap && totalBalance > 0 && (
          <button
            onClick={() => this.setState({ showNutzapModal: true })}
            class="shrink-0 p-1.5 text-green-400 hover:text-green-300 bg-green-400/10 hover:bg-green-400/20 rounded transition-colors"
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
            <div class="fixed top-1/2 left-1/2 transform -translate-x-1/2 -translate-y-1/2 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)] p-6 z-50 w-96 max-w-[90vw]">
              <h3 class="text-lg font-semibold mb-4">Send Nutzap</h3>
              
              <div class="space-y-4">
                <div>
                  <label class="block text-sm text-[var(--color-text-secondary)] mb-1">
                    Amount (sats)
                  </label>
                  <input
                    type="number"
                    value={amount}
                    onInput={(e) => this.setState({ amount: (e.target as HTMLInputElement).value })}
                    placeholder="100"
                    class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)] rounded-md text-[var(--color-text-primary)]"
                    disabled={sending}
                  />
                  <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
                    Balance: {totalBalance} sats
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
                    class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)] rounded-md text-[var(--color-text-primary)] resize-none"
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
                    class="flex-1 bg-green-600 hover:bg-green-700 disabled:bg-gray-600 px-4 py-2 rounded-md text-sm font-medium transition-colors disabled:cursor-not-allowed"
                  >
                    {sending ? 'Sending...' : 'Send Nutzap'}
                  </button>
                  <button
                    onClick={() => this.setState({ showNutzapModal: false })}
                    disabled={sending}
                    class="px-4 py-2 bg-[var(--color-bg-secondary)] hover:bg-[var(--color-bg-tertiary)] rounded-md text-sm font-medium transition-colors"
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