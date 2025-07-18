import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Proof } from '@cashu/cashu-ts'
import { TIMEOUTS, MIN_NUTZAP_AMOUNT } from '../constants'

interface UserDisplayProps {
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
  // Optional: pre-fetched profile data from centralized state
  profileData?: {
    profile?: any
    has10019?: boolean
  }
  // Balance passed down from ProfileMenu to avoid duplicate subscriptions
  walletBalance?: number
  hasWalletBalance?: boolean
  groupId?: string  // Optional group context for nutzaps
}

interface UserDisplayState {
  showNutzapModal: boolean
  sending: boolean
  amount: string
  comment: string
  error: string | null
  walletBalance: number
  selectedMint: string
  mintBalances: Record<string, number>
  canSendNutzap: boolean
  compatibleBalance: number
  compatibleMints: string[]
  incompatibilityReason?: string
  // UserDisplay state
  profilePicture: string | null
  displayId: string
  displayName: string | null
  copied: boolean
}

export class UserDisplay extends Component<UserDisplayProps, UserDisplayState> {
  private copyTimeout: number | null = null;
  private unsubscribeBalance: (() => void) | null = null;
  private compatibilityCheckTimer: number | null = null;
  private compatibilityCheckAttempts: number = 0;
  private unsubscribeWalletInit: (() => void) | null = null;

  state = {
    showNutzapModal: false,
    sending: false,
    amount: '',
    comment: '',
    error: null,
    walletBalance: 0,
    selectedMint: '',
    mintBalances: {},
    canSendNutzap: false,
    compatibleBalance: 0,
    compatibleMints: [],
    incompatibilityReason: undefined,
    // UserDisplay state
    profilePicture: null,
    displayId: '',
    displayName: null,
    copied: false
  }

  async componentDidMount() {
    // Add ESC key listener
    document.addEventListener('keydown', this.handleKeyDown);
    
    // Subscribe to balance updates for wallet state changes
    const { client } = this.props;
    if (client) {
      this.unsubscribeBalance = client.onBalanceUpdate((balance) => {
        this.setState({ walletBalance: balance });
      });
      
      // Subscribe to wallet initialization
      this.unsubscribeWalletInit = client.onWalletInitialized(() => {
        this.compatibilityCheckAttempts = 0; // Reset attempts
        this.checkRecipientCompatibility();
      });
    }
    
    // Fetch user profile
    const { pubkey, profileData } = this.props
    
    // Convert to npub if it's a hex pubkey
    const displayId = pubkey.startsWith('npub') ? pubkey : client?.pubkeyToNpub(pubkey) || pubkey
    this.setState({ displayId })
    
    // Check if we have pre-fetched profile data
    if (profileData?.profile) {
      const profile = profileData.profile;
      if (profile.image) {
        this.setState({ profilePicture: profile.image })
      }
      // Set display name in order of preference: NIP-05 > Name > null
      const displayName = profile.nip05 || profile.name || profile.display_name || null
      this.setState({ displayName })
      
      // Also use pre-fetched 10019 status if available
      if (profileData.has10019 !== undefined) {
        // Pre-fetched profile data available, but we still need full compatibility check
        // since has10019 just means they have the config, not that we're compatible
        this.scheduleCompatibilityCheck();
      } else {
        // Check compatibility if not pre-fetched
        this.scheduleCompatibilityCheck();
      }
    } else if (client) {
      // Fallback to fetching profile if not pre-fetched
      const hexPubkey = pubkey.startsWith('npub') ? client.npubToPubkey(pubkey) : pubkey
      const profile = await client.fetchProfile(hexPubkey)
      if (profile) {
        if (profile.image) {
          this.setState({ profilePicture: profile.image })
        }
        // Set display name in order of preference: NIP-05 > Name > null
        const displayName = profile.nip05 || profile.name || profile.display_name || null
        this.setState({ displayName })
      }
      
      // Check if we can send nutzaps to this recipient
      this.scheduleCompatibilityCheck();
    }
  }

  componentDidUpdate(prevProps: UserDisplayProps) {
    // Re-check if pubkey changes
    if (prevProps.pubkey !== this.props.pubkey) {
      this.compatibilityCheckAttempts = 0; // Reset attempts for new user
      this.checkRecipientCompatibility();
    }
    
    // Also re-check if wallet balance changed (indicating wallet state change)
    if (prevProps.walletBalance !== this.props.walletBalance && this.props.walletBalance !== undefined) {
      // Clear recipient's cache and re-check when our balance changes
      // This helps when recipient might have added mints
      const { pubkey, client } = this.props;
      const hexPubkey = pubkey.startsWith('npub') ? client.npubToPubkey(pubkey) : pubkey;
      const walletService = client.getWalletService();
      if (walletService && hexPubkey) {
        walletService.clearUser10019Cache(hexPubkey);
        this.checkRecipientCompatibility();
      }
    }
    
    // Re-check if wallet initialization status changes
    if (prevProps.hasWalletBalance !== this.props.hasWalletBalance && this.props.hasWalletBalance) {
      this.checkRecipientCompatibility();
    }
  }

  componentWillUnmount() {
    // Remove ESC key listener
    document.removeEventListener('keydown', this.handleKeyDown);
    // Unsubscribe from balance updates
    if (this.unsubscribeBalance) {
      this.unsubscribeBalance();
    }
    // Unsubscribe from wallet init
    if (this.unsubscribeWalletInit) {
      this.unsubscribeWalletInit();
    }
    // Clear copy timeout
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }
    // Clear compatibility check timer
    if (this.compatibilityCheckTimer) {
      window.clearTimeout(this.compatibilityCheckTimer)
    }
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

  // Schedule compatibility check with retry logic
  scheduleCompatibilityCheck = () => {
    // Clear any existing timer
    if (this.compatibilityCheckTimer) {
      window.clearTimeout(this.compatibilityCheckTimer);
    }
    
    // Schedule the check
    this.compatibilityCheckTimer = window.setTimeout(() => {
      this.checkRecipientCompatibility();
    }, TIMEOUTS.COMPATIBILITY_CHECK_DELAY);
  }

  // Check if target user has a kind:10019 event (nutzap config)
  checkRecipientCompatibility = async () => {
    try {
      const { pubkey, client } = this.props
      // Convert npub to hex if needed
      const hexPubkey = pubkey.startsWith('npub') ? client.npubToPubkey(pubkey) : pubkey

      // Check if wallet is initialized first
      if (!client.isWalletInitialized()) {
        
        // Retry up to 10 times with exponential backoff
        if (this.compatibilityCheckAttempts < 10) {
          this.compatibilityCheckAttempts++;
          const delay = Math.min(1000 * Math.pow(1.5, this.compatibilityCheckAttempts), 10000);
          
          this.compatibilityCheckTimer = window.setTimeout(() => {
            this.checkRecipientCompatibility();
          }, delay);
        } else {
          this.setState({ 
            canSendNutzap: false,
            incompatibilityReason: "Wallet initialization timeout"
          });
        }
        return;
      }

      // Use gossip model to check compatibility
      const walletService = client.getWalletService();
      if (!walletService) {
        this.setState({ 
          canSendNutzap: false,
          incompatibilityReason: "Wallet service not available"
        })
        return
      }
      
      // Check compatibility using the new method
      const compatibility = await walletService.canSendToRecipient(hexPubkey, MIN_NUTZAP_AMOUNT)
      
      this.setState({ 
        canSendNutzap: compatibility.canSend,
        compatibleBalance: compatibility.compatibleBalance,
        compatibleMints: compatibility.compatibleMints,
        incompatibilityReason: compatibility.reason
      })
      
      // Reset attempts counter on success
      this.compatibilityCheckAttempts = 0;
      
    } catch (error) {
      this.setState({ 
        canSendNutzap: false,
        incompatibilityReason: "Error checking compatibility"
      })
    }
  }

  // Fetch wallet balance and mint balances when modal opens
  fetchWalletBalance = async () => {
    try {
      const { pubkey, client } = this.props
      const hexPubkey = pubkey.startsWith('npub') ? client.npubToPubkey(pubkey) : pubkey

      // Get compatible mint balances only
      const walletService = client.getWalletService()
      if (walletService) {
        const compatibleMintBalances = await walletService.getCompatibleMintsWithBalances(hexPubkey)

        // Auto-select the first compatible mint with highest balance
        const sortedMints = Object.entries(compatibleMintBalances)
          .filter(([_, balance]) => balance > 0)
          .sort(([, a], [, b]) => b - a)
        
        const selectedMint = sortedMints.length > 0 ? sortedMints[0][0] : ''

        // IMPORTANT: Keep walletBalance as the compatible balance for this modal
        // This is what the user can actually send to this recipient
        this.setState({
          walletBalance: this.state.compatibleBalance, // Use stored compatible balance
          mintBalances: compatibleMintBalances,
          selectedMint
        })
      } else {
        this.setState({
          walletBalance: this.state.compatibleBalance, // Use stored compatible balance
          mintBalances: {},
          selectedMint: ''
        })
      }
    } catch (error) {
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
      if (totalBalance === 0) {
        // Check if we might have tokens in unauthorized mints
        const allMintBalances = Object.values(this.state.mintBalances as Record<string, number>).reduce((sum, balance) => sum + balance, 0);
        if (allMintBalances > 0) {
          this.setState({ 
            error: `No balance available in recipient's accepted mints. You have ${allMintBalances} sats in other mints, but the recipient only accepts specific mints. Check console for details.` 
          })
        } else {
          this.setState({ error: `Insufficient balance. You have ${totalBalance} sats but tried to send ${sats} sats.` })
        }
      } else {
        this.setState({ error: `Insufficient balance. You have ${totalBalance} sats but tried to send ${sats} sats.` })
      }
      return
    }

    this.setState({ sending: true, error: null })

    try {
      // Convert npub to hex if needed
      const hexPubkey = pubkey.startsWith('npub') ? client.npubToPubkey(pubkey) : pubkey

      // No timeout needed - the CashuWalletService now handles this with NDK zapper events
      await client.sendNutzap(hexPubkey, sats, undefined, this.props.groupId);

      // SUCCESS - Close modal and refresh balance
      this.setState({
        showNutzapModal: false,
        amount: '',
        comment: '',
        error: null
      })

      // Don't update compatible balance as total balance - let the wallet service handle the actual balance update
      // The balance callbacks will update the UI with the correct total balance

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

  handleCopy = () => {
    const { onCopy } = this.props
    const { displayId } = this.state

    navigator.clipboard.writeText(displayId)
    this.setState({ copied: true })
    if (onCopy) onCopy()

    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }

    this.copyTimeout = window.setTimeout(() => {
      this.setState({ copied: false })
    }, TIMEOUTS.COPY_FEEDBACK_DURATION)
  }

  getSizeClasses() {
    switch (this.props.size || 'md') {
      case 'sm':
        return {
          container: 'gap-1.5',
          image: 'h-6 w-6',
          text: 'text-xs',
          copyIcon: 'w-3 h-3'
        }
      case 'lg':
        return {
          container: 'gap-3',
          image: 'h-10 w-10',
          text: 'text-base',
          copyIcon: 'w-4 h-4'
        }
      default:
        return {
          container: 'gap-2',
          image: 'h-8 w-8',
          text: 'text-sm',
          copyIcon: 'w-3.5 h-3.5'
        }
    }
  }

  truncateId(id: string): string {
    if (!id) return ''
    return `${id.slice(0, 8)}...${id.slice(-4)}`
  }

  render() {
    const { pubkey, client, showCopy = true, isRelayAdmin = false, hideNutzap } = this.props
    const { showNutzapModal, sending, amount, comment, error, profilePicture, displayId, displayName, copied } = this.state
    const sizeClasses = this.getSizeClasses()

    // Use balance passed from ProfileMenu or fallback to client check
    const hasWalletBalance = this.props.hasWalletBalance ?? client.hasWalletBalance()

    return (
      <div class="flex items-center gap-2">
        {/* UserDisplay content inline */}
        <div class={`flex items-center ${sizeClasses.container}`}>
          <div class={`shrink-0 ${sizeClasses.image} rounded-full bg-[var(--color-bg-secondary)] border border-[var(--color-border)] overflow-hidden relative`}>
            {profilePicture ? (
              <img
                src={profilePicture}
                alt=""
                class="w-full h-full object-cover"
                onError={(e) => {
                  (e.target as HTMLImageElement).style.display = 'none'
                  const parent = e.currentTarget.parentElement
                  if (parent) {
                    const fallback = document.createElement('div')
                    fallback.className = 'w-full h-full flex items-center justify-center text-sm font-medium text-[var(--color-text-secondary)]'
                    fallback.textContent = pubkey.slice(0, 2).toUpperCase()
                    parent.appendChild(fallback)
                  }
                }}
              />
            ) : (
              <div class="w-full h-full flex items-center justify-center text-sm font-medium text-[var(--color-text-secondary)]">
                {pubkey.slice(0, 2).toUpperCase()}
              </div>
            )}
          </div>
          <div class={`truncate ${sizeClasses.text} text-[var(--color-text-primary)] flex items-center gap-1.5`}>
            <span title={displayId}>{displayName || this.truncateId(displayId)}</span>
            {isRelayAdmin && (
              <span class="shrink-0 px-1.5 py-0.5 text-[10px] font-medium bg-yellow-500/10 text-yellow-500 rounded-full border border-yellow-500/20">
                Relay Admin
              </span>
            )}
            {showCopy && (
              <button
                onClick={this.handleCopy}
                class="opacity-0 group-hover:opacity-100 text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-all"
                title={copied ? "Copied!" : "Copy npub"}
              >
                {copied ? (
                  <svg class={sizeClasses.copyIcon} viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d="M20 6L9 17L4 12" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                ) : (
                  <svg class={sizeClasses.copyIcon} viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d="M8 4v12a2 2 0 002 2h8a2 2 0 002-2V7.242a2 2 0 00-.602-1.43L16.083 2.57A2 2 0 0014.685 2H10a2 2 0 00-2 2z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    <path d="M16 18v2a2 2 0 01-2 2H6a2 2 0 01-2-2V9a2 2 0 012-2h2" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                )}
              </button>
            )}
          </div>
        </div>

        {!hideNutzap && hasWalletBalance && (
          this.state.canSendNutzap ? (
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
          ) : (
            <button
              disabled
              class="shrink-0 p-1.5 text-gray-500 bg-gray-500/10 rounded opacity-50 cursor-not-allowed"
              title={this.state.incompatibilityReason || "Unable to send nutzap"}
            >
              <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </button>
          )
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
                    Compatible Mints
                  </label>
                  <div class="text-xs text-[var(--color-text-tertiary)] space-y-1">
                    {Object.entries(this.state.mintBalances).map(([mint, balance]) => (
                      <div key={mint} class="flex items-center gap-2">
                        <span>•</span>
                        <span>{new URL(mint).hostname}</span>
                        <span class="text-[#f7931a] font-medium">₿{(balance as number).toLocaleString()} sats</span>
                      </div>
                    ))}
                  </div>
                  <p class="text-xs text-[var(--color-text-tertiary)] mt-2">
                    You can send up to <span class="text-[#f7931a] font-medium">₿{this.state.walletBalance.toLocaleString()} sats</span> from compatible mints.
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
                  <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
                    Available: <span class="text-[#f7931a] font-medium">₿{this.state.walletBalance.toLocaleString()} sats</span>
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