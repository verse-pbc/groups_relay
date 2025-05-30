import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { UserDisplay } from './UserDisplay'
import { BaseComponent } from './BaseComponent'
import type { Proof } from '@cashu/cashu-ts'

interface ContentSectionProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  cashuProofs?: Proof[]
  mints?: string[]
  onNutzapSent?: () => void
}

interface ContentSectionState {
  deletingEvents: Set<string>
  showConfirmDelete: string | null
  showNutzapModal: string | null
  nutzapAmount: string
  nutzapLoading: boolean
  nutzapError: string | null
  walletBalance: number
  eventNutzaps: Map<string, number> // eventId -> total amount
  authorHas10019: Map<string, boolean> // pubkey -> has 10019 event
}

export class ContentSection extends BaseComponent<ContentSectionProps, ContentSectionState> {
  state = {
    deletingEvents: new Set<string>(),
    showConfirmDelete: null,
    showNutzapModal: null,
    nutzapAmount: '',
    nutzapLoading: false,
    nutzapError: null,
    walletBalance: 0,
    eventNutzaps: new Map<string, number>(),
    authorHas10019: new Map<string, boolean>()
  }

  private nutzapSubscription: any = null
  private authorMints: Map<string, string[]> | undefined
  private eventAuthors: Map<string, string> | undefined
  private authorCashuPubkeys: Map<string, string> | undefined

  componentDidMount() {
    this.subscribeToNutzaps()
    // Add ESC key listener
    document.addEventListener('keydown', this.handleKeyDown)
  }

  componentWillUnmount() {
    if (this.nutzapSubscription) {
      this.nutzapSubscription.stop()
    }
    // Remove ESC key listener
    document.removeEventListener('keydown', this.handleKeyDown)
  }

  handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      if (this.state.showNutzapModal) {
        this.setState({ showNutzapModal: null, nutzapError: null })
      }
      if (this.state.showConfirmDelete) {
        this.setState({ showConfirmDelete: null })
      }
    }
  }

  componentDidUpdate(prevProps: ContentSectionProps) {
    if (prevProps.group.id !== this.props.group.id) {
      // Re-subscribe when group changes
      if (this.nutzapSubscription) {
        this.nutzapSubscription.stop()
      }
      this.setState({ 
        eventNutzaps: new Map(),
        authorHas10019: new Map()
      })
      this.subscribeToNutzaps()
    }
  }

  private subscribeToNutzaps = async () => {
    try {
      // Get all event IDs from the current group content
      const eventIds = this.props.group.content?.map(item => item.id) || []
      if (eventIds.length === 0) return

      // Subscribe to nutzaps for these events
      // Note: Nutzaps are tagged with the regular nostr pubkey in the 'p' tag,
      // not the cashu P2PK pubkey. The cashu pubkey is only used in the proof.
      const filter = {
        kinds: [9321], // NIP-61 nutzap kind
        '#e': eventIds // Events that are referenced
      }

      // Build a map of eventId -> authorPubkey for later lookups
      const eventAuthors = new Map<string, string>()
      this.props.group.content?.forEach(item => {
        eventAuthors.set(item.id, item.pubkey)
      })

      // Get unique authors to fetch their 10019 events
      const uniqueAuthors = [...new Set(eventAuthors.values())]
      
      // Fetch 10019 events for all content authors
      const authorMints = new Map<string, string[]>()
      const authorHas10019 = new Map<string, boolean>()
      const authorCashuPubkeys = new Map<string, string>() // Map author pubkey -> cashu P2PK pubkey
      
      if (uniqueAuthors.length > 0) {
        const authorFilter = {
          kinds: [10019],
          authors: uniqueAuthors,
          limit: uniqueAuthors.length
        }
        
        // Use profileNdk to fetch 10019 events from public relays
        const profileNdk = (this.props.client as any).profileNdk
        const author10019EventsSet = await profileNdk.fetchEvents(authorFilter)
        const author10019Events = Array.from(author10019EventsSet)
        
        console.log(`Fetched ${author10019Events.length} kind:10019 events for ${uniqueAuthors.length} authors`)
        
        // Initialize all authors as not having 10019
        uniqueAuthors.forEach(author => {
          authorHas10019.set(author, false)
        })
        
        author10019Events.forEach((event: any) => {
          const mints: string[] = []
          let cashuPubkey: string | null = null
          
          event.tags.forEach((tag: string[]) => {
            if (tag[0] === 'mint' && tag[1]) {
              // Normalize mint URL (remove trailing slash)
              const normalizedMint = tag[1].replace(/\/$/, '')
              mints.push(normalizedMint)
            } else if (tag[0] === 'pubkey' && tag[1]) {
              // This is the P2PK pubkey for receiving nutzaps
              cashuPubkey = tag[1]
            }
          })
          
          if (mints.length > 0 && cashuPubkey) {
            authorMints.set(event.pubkey, mints)
            authorHas10019.set(event.pubkey, true)
            authorCashuPubkeys.set(event.pubkey, cashuPubkey)
          }
        })
      }

      // Fetch existing nutzaps from public relays
      const profileNdk = (this.props.client as any).profileNdk
      const eventsSet = await profileNdk.fetchEvents(filter)
      const events = Array.from(eventsSet)
      const nutzapTotals = new Map<string, number>()

      events.forEach((event: any) => {
        // Find the event tag
        const eventTag = event.tags.find((tag: string[]) => tag[0] === 'e')
        if (!eventTag) return

        const eventId = eventTag[1]
        
        // Get the author of the target event
        const eventAuthor = eventAuthors.get(eventId)
        if (!eventAuthor) return
        
        // Get the authorized mints for this event's author
        const authorizedMints = authorMints.get(eventAuthor) || []
        const cashuPubkey = authorCashuPubkeys.get(eventAuthor)
        
        // Skip if author has no 10019 config
        if (authorizedMints.length === 0 || !cashuPubkey) return
        
        // Find the amount tag
        const amountTag = event.tags.find((tag: string[]) => tag[0] === 'amount')
        if (!amountTag) return

        const amount = parseInt(amountTag[1])
        if (isNaN(amount)) return

        // Find the mint tag - check 'u' tag as per NIP-61 
        const uTag = event.tags.find((tag: string[]) => tag[0] === 'u')
        const mint = uTag ? uTag[1] : null

        // According to NIP-61, nutzaps MUST have a 'u' tag with the mint URL
        if (!mint) return
        
        // Normalize mint URL for comparison (remove trailing slash)
        const normalizedMint = mint.replace(/\/$/, '')
        const normalizedAuthorizedMints = authorizedMints.map(m => m.replace(/\/$/, ''))
        
        // Verify mint is authorized
        if (!normalizedAuthorizedMints.includes(normalizedMint)) return
        
        // Find the proof tag and verify P2PK lock
        const proofTag = event.tags.find((tag: string[]) => tag[0] === 'proof')
        if (!proofTag) return
        
        try {
          const proof = JSON.parse(proofTag[1])
          // Parse the secret to check P2PK lock
          const secret = JSON.parse(proof.secret)
          
          // Verify it's P2PK locked
          if (!Array.isArray(secret) || secret[0] !== 'P2PK') return
          
          // Extract the locked pubkey (should have "02" prefix for Cashu)
          const lockedPubkey = secret[1]?.data
          if (!lockedPubkey) return
          
          // Verify it matches the cashu pubkey from kind:10019
          // The cashuPubkey from kind:10019 might not have the "02" prefix
          const normalizedCashuPubkey = cashuPubkey.startsWith('02') ? cashuPubkey : '02' + cashuPubkey
          const normalizedLockedPubkey = lockedPubkey.startsWith('02') ? lockedPubkey : '02' + lockedPubkey
          
          if (normalizedCashuPubkey !== normalizedLockedPubkey) return
          
          // All verifications passed - count this nutzap
          const currentTotal = nutzapTotals.get(eventId) || 0
          nutzapTotals.set(eventId, currentTotal + amount)
        } catch (e) {
          // Invalid proof format, skip
          return
        }
      })
      
      this.setState({ 
        eventNutzaps: nutzapTotals,
        authorHas10019: authorHas10019
      })

      // Store the maps for the subscription handler
      this.authorMints = authorMints
      this.eventAuthors = eventAuthors
      this.authorCashuPubkeys = authorCashuPubkeys

      // Subscribe to new nutzaps from public relays
      const profileNdkForSub = (this.props.client as any).profileNdk
      this.nutzapSubscription = await profileNdkForSub.subscribe(filter, {
        closeOnEose: false
      })

      this.nutzapSubscription.on('event', (event: any) => {
        // Find the event tag
        const eventTag = event.tags.find((tag: string[]) => tag[0] === 'e')
        if (!eventTag) return

        const eventId = eventTag[1]
        
        // Get the author of the target event
        const eventAuthor = this.eventAuthors?.get(eventId)
        if (!eventAuthor) return
        
        // Get the authorized mints for this event's author
        const authorizedMints = this.authorMints?.get(eventAuthor) || []
        const cashuPubkey = this.authorCashuPubkeys?.get(eventAuthor)
        
        // Skip if author has no 10019 config
        if (authorizedMints.length === 0 || !cashuPubkey) return
        
        // Find the amount tag
        const amountTag = event.tags.find((tag: string[]) => tag[0] === 'amount')
        if (!amountTag) return

        const amount = parseInt(amountTag[1])
        if (isNaN(amount)) return

        // Find the mint tag - check 'u' tag as per NIP-61
        const uTag = event.tags.find((tag: string[]) => tag[0] === 'u')
        const mint = uTag ? uTag[1] : null

        // According to NIP-61, nutzaps MUST have a 'u' tag
        if (!mint) return
        
        // Normalize mint URL for comparison (remove trailing slash)
        const normalizedMint = mint.replace(/\/$/, '')
        
        // Verify mint is authorized (authorizedMints are already normalized)
        if (!authorizedMints.includes(normalizedMint)) return
        
        // Find the proof tag and verify P2PK lock
        const proofTag = event.tags.find((tag: string[]) => tag[0] === 'proof')
        if (!proofTag) return
        
        try {
          const proof = JSON.parse(proofTag[1])
          // Parse the secret to check P2PK lock
          const secret = JSON.parse(proof.secret)
          
          // Verify it's P2PK locked
          if (!Array.isArray(secret) || secret[0] !== 'P2PK') return
          
          // Extract the locked pubkey (should have "02" prefix for Cashu)
          const lockedPubkey = secret[1]?.data
          if (!lockedPubkey) return
          
          // Verify it matches the cashu pubkey from kind:10019
          const normalizedCashuPubkey = cashuPubkey.startsWith('02') ? cashuPubkey : '02' + cashuPubkey
          const normalizedLockedPubkey = lockedPubkey.startsWith('02') ? lockedPubkey : '02' + lockedPubkey
          
          if (normalizedCashuPubkey !== normalizedLockedPubkey) return
          
          // All verifications passed - count this nutzap
          this.setState(prev => {
            const newTotals = new Map(prev.eventNutzaps)
            const currentTotal = newTotals.get(eventId) || 0
            newTotals.set(eventId, currentTotal + amount)
            return { eventNutzaps: newTotals }
          })
        } catch (e) {
          // Invalid proof format, skip
          return
        }
      })
    } catch (error) {
      console.error('Failed to subscribe to nutzaps:', error)
    }
  }

  handleDeleteEvent = async (eventId: string) => {
    this.setState(prev => ({
      deletingEvents: new Set(prev.deletingEvents).add(eventId),
      showConfirmDelete: null
    }))

    try {
      await this.props.client.deleteEvent(this.props.group.id, eventId)
      this.props.group.content = this.props.group.content?.filter(item => item.id !== eventId) || []
      this.props.showMessage('Event deleted successfully', 'success')
    } catch (error) {
      console.error('Failed to delete event:', error)
      this.showError('Failed to delete event', error)
    } finally {
      this.setState(prev => {
        const newSet = new Set(prev.deletingEvents)
        newSet.delete(eventId)
        return { deletingEvents: newSet }
      })
    }
  }

  getCurrentUserPubkey = (): string | null => {
    try {
      const signer = this.props.client.ndkInstance?.signer;
      if (!signer) return null;
      // Get the user synchronously if possible
      const user = (signer as any)._user;
      return user?.pubkey || null;
    } catch {
      return null;
    }
  }

  fetchWalletBalance = async () => {
    try {
      console.log('🔍 [NUTZAP] Fetching current wallet balance...')
      const balance = await this.props.client.getCashuBalance()
      console.log('🔍 [NUTZAP] Fetched balance:', balance)
      this.setState({ walletBalance: balance })
    } catch (error) {
      console.error('❌ [NUTZAP] Failed to fetch wallet balance:', error)
      this.setState({ walletBalance: 0 })
    }
  }

  handleSendEventNutzap = async (eventId: string) => {
    const { nutzapAmount, walletBalance } = this.state
    
    const sats = parseInt(nutzapAmount)
    if (!sats || sats <= 0) {
      this.setState({ nutzapError: 'Please enter a valid amount' })
      return
    }

    // Check if user has sufficient balance
    console.log('🔍 [NUTZAP] Balance validation - Attempting to send:', sats, 'Available balance:', walletBalance)
    if (sats > walletBalance) {
      this.setState({ nutzapError: `Insufficient balance. You have ${walletBalance} sats but tried to send ${sats} sats.` })
      return
    }

    this.setState({ nutzapLoading: true, nutzapError: null })

    try {
      await this.props.client.sendNutzapToEvent(eventId, sats)
      
      // SUCCESS - Update balance optimistically (without re-fetching from mints)
      const newBalance = Math.max(0, walletBalance - sats)
      console.log('💸 [NUTZAP] Success! Optimistic balance update - Old:', walletBalance, 'New:', newBalance, 'Sent:', sats)
      
      // Notify other components immediately
      this.props.client.notifyBalanceUpdate(newBalance)
      
      // Update local state with new balance and nutzap totals
      this.setState(prev => {
        const newTotals = new Map(prev.eventNutzaps)
        const currentTotal = newTotals.get(eventId) || 0
        newTotals.set(eventId, currentTotal + sats)
        
        return {
          eventNutzaps: newTotals,
          showNutzapModal: null, 
          nutzapAmount: '', 
          nutzapError: null,
          walletBalance: newBalance
        }
      })
      
      this.props.showMessage('Nutzap sent to event successfully!', 'success')
      if (this.props.onNutzapSent) this.props.onNutzapSent()
    } catch (error) {
      // Error - just show the error, no balance changes needed
      console.log('🔴 [NUTZAP] Error:', error)
      this.setState({ 
        nutzapError: error instanceof Error ? error.message : 'Failed to send nutzap' 
      })
    } finally {
      this.setState({ nutzapLoading: false })
    }
  }

  formatTimestamp = (timestamp: number) => {
    const date = new Date(timestamp * 1000)
    const now = new Date()
    const diffInSeconds = Math.floor((now.getTime() - date.getTime()) / 1000)

    // Less than a minute ago
    if (diffInSeconds < 60) {
      return 'just now'
    }

    // Less than an hour ago
    if (diffInSeconds < 3600) {
      const minutes = Math.floor(diffInSeconds / 60)
      return `${minutes}m ago`
    }

    // Less than a day ago
    if (diffInSeconds < 86400) {
      const hours = Math.floor(diffInSeconds / 3600)
      return `${hours}h ago`
    }

    // Less than a week ago
    if (diffInSeconds < 604800) {
      const days = Math.floor(diffInSeconds / 86400)
      return `${days}d ago`
    }

    // If it's this year
    if (date.getFullYear() === now.getFullYear()) {
      return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
    }

    // If it's a different year
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
  }

  render() {
    const { group, client } = this.props
    const { deletingEvents, showConfirmDelete, showNutzapModal, nutzapAmount, nutzapLoading, nutzapError, walletBalance } = this.state
    const content = group.content || []

    // Get wallet state from client
    const cashuProofs = client.getCashuProofs()
    const mints = client.getWalletMints()
    const hasWalletBalance = client.hasWalletBalance()

    return (
      <div class="h-full flex flex-col overflow-hidden">
        <div class="flex-1 overflow-y-auto">
          <div class="space-y-3">
            {content.map((item) => (
              <div
                key={item.id}
                class="group p-2 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]
                       hover:border-[var(--color-border-hover)] transition-colors relative"
              >
                <div class="flex items-start gap-1.5">
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center text-[11px] gap-1.5 text-[var(--color-text-tertiary)]">
                      <UserDisplay
                        pubkey={this.props.client.pubkeyToNpub(item.pubkey)}
                        client={this.props.client}
                        showCopy={true}
                        size="sm"
                        onCopy={() => this.props.showMessage('Npub copied to clipboard', 'success')}
                        cashuProofs={cashuProofs}
                        mints={mints}
                        onSendNutzap={() => {
                          this.props.showMessage('Nutzap sent successfully!', 'success');
                          if (this.props.onNutzapSent) this.props.onNutzapSent();
                        }}
                        hideNutzap={item.pubkey === this.getCurrentUserPubkey() && !window.location.search.includes('selfnutzap')}
                      />
                      <span>·</span>
                      <span>
                        {this.formatTimestamp(item.created_at)}
                      </span>
                      {/* Nutzap total */}
                      {this.state.eventNutzaps.get(item.id) && (
                        <>
                          <span>·</span>
                          <span class="text-[#f7931a] flex items-center gap-0.5 font-medium">
                            <svg class="w-3 h-3" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" fill="currentColor"/>
                            </svg>
                            ₿{this.state.eventNutzaps.get(item.id)?.toLocaleString() || 0} sats
                          </span>
                        </>
                      )}
                    </div>
                    <p class="text-sm text-[var(--color-text-primary)] break-all whitespace-pre-wrap leading-relaxed mt-0.5">
                      {item.content}
                    </p>
                  </div>

                  <div class="flex items-center gap-1">
                    {/* Event Nutzap Button and Total */}
                    {((hasWalletBalance && 
                      item.pubkey !== this.getCurrentUserPubkey() && 
                      this.state.authorHas10019.get(item.pubkey)) || 
                      this.state.eventNutzaps.get(item.id)) && (
                      <div class="flex items-center gap-1">
                        {hasWalletBalance && 
                         item.pubkey !== this.getCurrentUserPubkey() && 
                         this.state.authorHas10019.get(item.pubkey) && (
                          <button
                            onClick={async () => {
                              this.setState({ showNutzapModal: item.id })
                              await this.fetchWalletBalance()
                            }}
                            class="text-[11px] opacity-0 group-hover:opacity-100 text-[#f7931a]
                                   hover:text-[#f68e0a] transition-all duration-150 flex items-center p-1"
                            title="Nutzap this message"
                          >
                            <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                            </svg>
                          </button>
                        )}
                        {this.state.eventNutzaps.get(item.id) ? (
                          <span class="text-[10px] text-[#f7931a] font-medium">
                            ₿ {this.state.eventNutzaps.get(item.id)?.toLocaleString()} sats
                          </span>
                        ) : null}
                      </div>
                    )}

                    {/* Delete Button */}
                    {showConfirmDelete === item.id ? (
                      <div class="flex items-center gap-1 text-[11px]">
                        <button
                          onClick={() => this.handleDeleteEvent(item.id)}
                          class="text-red-400 hover:text-red-300 transition-colors"
                        >
                          Delete
                        </button>
                        <span class="text-[var(--color-text-tertiary)]">·</span>
                        <button
                          onClick={() => this.setState({ showConfirmDelete: null })}
                          class="text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                        >
                          Cancel
                        </button>
                      </div>
                    ) : (
                      <button
                        onClick={() => this.setState({ showConfirmDelete: item.id })}
                        disabled={deletingEvents.has(item.id)}
                        class="text-[11px] opacity-0 group-hover:opacity-100 text-red-400
                               hover:text-red-300 transition-all duration-150 flex items-center"
                        title="Delete message"
                      >
                        {deletingEvents.has(item.id) ? (
                          <span class="animate-spin">⚡</span>
                        ) : (
                          <svg class="w-3.5 h-3.5 text-red-400" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                            <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                            <path d="M10 11v6M14 11v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                          </svg>
                        )}
                      </button>
                    )}
                  </div>
                </div>
              </div>
            ))}

            {content.length === 0 && (
              <div class="text-center py-12">
                <div class="mb-3 text-2xl">💭</div>
                <p class="text-sm text-[#8484ac]">No activity yet</p>
                <p class="text-xs text-[#8484ac] mt-1">
                  Messages will appear here when members start posting
                </p>
              </div>
            )}
          </div>
        </div>

        {/* Event Nutzap Modal */}
        {showNutzapModal && (
          <>
            {/* Modal backdrop */}
            <div 
              class="fixed inset-0 bg-black/50 z-50" 
              onClick={() => this.setState({ showNutzapModal: null, nutzapError: null })}
            />
            
            {/* Modal */}
            <div class="fixed top-1/2 left-1/2 transform -translate-x-1/2 -translate-y-1/2 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)] p-6 z-50 w-96 max-w-[90vw]">
              <h3 class="text-lg font-semibold mb-4">Nutzap Event</h3>
              
              <div class="space-y-4">
                {/* Balance display */}
                <div class="text-sm text-[var(--color-text-secondary)]">
                  Balance: <span class="text-[#f7931a] font-medium">₿{walletBalance.toLocaleString()} sats</span>
                </div>

                {/* Amount input */}
                <div>
                  <label class="block text-sm font-medium text-[var(--color-text-secondary)] mb-1">
                    Amount (sats)
                  </label>
                  <input
                    type="number"
                    value={nutzapAmount}
                    onInput={(e: any) => this.setState({ nutzapAmount: e.target.value, nutzapError: null })}
                    placeholder="Enter amount"
                    class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)] rounded text-[var(--color-text-primary)] focus:outline-none focus:border-green-400"
                    disabled={nutzapLoading}
                  />
                </div>

                {/* Preset amounts */}
                <div class="flex gap-2 flex-wrap">
                  {[21, 100, 500].map(preset => (
                    <button
                      key={preset}
                      onClick={() => this.setState({ nutzapAmount: preset.toString(), nutzapError: null })}
                      class="px-3 py-1 text-sm bg-[var(--color-bg-secondary)] hover:bg-[#f7931a]/10 text-[var(--color-text-secondary)] hover:text-[#f7931a] rounded transition-colors"
                      disabled={nutzapLoading}
                    >
                      <span class="text-[#f7931a]">₿{preset}</span> sats
                    </button>
                  ))}
                </div>

                {/* Error message */}
                {nutzapError && (
                  <div class="text-sm text-red-400">
                    {nutzapError}
                  </div>
                )}

                {/* Action buttons */}
                <div class="flex gap-2 justify-end">
                  <button
                    onClick={() => this.setState({ showNutzapModal: null, nutzapError: null })}
                    class="px-4 py-2 text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
                    disabled={nutzapLoading}
                  >
                    Cancel
                  </button>
                  <button
                    onClick={() => this.handleSendEventNutzap(showNutzapModal)}
                    disabled={nutzapLoading || !nutzapAmount}
                    class="px-4 py-2 text-sm bg-[#f7931a] hover:bg-[#f68e0a] text-white rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
                  >
                    {nutzapLoading ? (
                      <>
                        <span class="animate-spin">⚡</span>
                        Sending...
                      </>
                    ) : (
                      <>
                        <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                          <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                        </svg>
                        Send Nutzap
                      </>
                    )}
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