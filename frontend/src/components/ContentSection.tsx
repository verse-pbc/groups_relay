import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { UserDisplay } from './UserDisplay'
import { BaseComponent } from './BaseComponent'
import type { Proof } from '@cashu/cashu-ts'
import { MIN_NUTZAP_AMOUNT } from '../constants'

interface ContentSectionProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  cashuProofs?: Proof[]
  mints?: string[]
  onNutzapSent?: () => void
  // Balance passed down from ProfileMenu to avoid duplicate subscriptions
  walletBalance?: number
  hasWalletBalance?: boolean
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
  authorCompatibility: Map<string, {
    canSend: boolean
    compatibleBalance: number
    compatibleMints: string[]
    reason?: string
  }> // pubkey -> compatibility info
}

export class ContentSection extends BaseComponent<ContentSectionProps, ContentSectionState> {
  private unsubscribeBalance: (() => void) | null = null;
  state = {
    deletingEvents: new Set<string>(),
    showConfirmDelete: null,
    showNutzapModal: null,
    nutzapAmount: '',
    nutzapLoading: false,
    nutzapError: null,
    walletBalance: 0,
    eventNutzaps: new Map<string, number>(),
    authorHas10019: new Map<string, boolean>(),
    authorCompatibility: new Map<string, {
      canSend: boolean
      compatibleBalance: number
      compatibleMints: string[]
      reason?: string
    }>()
  }

  private nutzapSubscription: any = null
  private authorMints: Map<string, string[]> | undefined
  private eventAuthors: Map<string, string> | undefined
  private authorCashuPubkeys: Map<string, string> | undefined

  componentDidMount() {
    this.subscribeToNutzaps()
    // Add ESC key listener
    document.addEventListener('keydown', this.handleKeyDown)
    
    // Subscribe to balance updates for wallet state changes
    const { client } = this.props;
    if (client) {
      this.unsubscribeBalance = client.onBalanceUpdate((balance) => {
        this.setState({ walletBalance: balance });
      });
    }
  }

  componentWillUnmount() {
    if (this.nutzapSubscription) {
      this.nutzapSubscription.stop()
    }
    // Unsubscribe from balance updates
    if (this.unsubscribeBalance) {
      this.unsubscribeBalance();
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
        authorHas10019: new Map(),
        authorCompatibility: new Map()
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
      
      // Fetch 10019 events for all content authors using gossip model
      const authorMints = new Map<string, string[]>()
      const authorHas10019 = new Map<string, boolean>()
      const authorCashuPubkeys = new Map<string, string>() // Map author pubkey -> cashu P2PK pubkey
      const allNutzapRelays = new Set<string>() // Collect all nutzap relays from kind:10019
      
      if (uniqueAuthors.length > 0) {
        // Use gossip model to fetch from users' write relays
        const walletService = this.props.client.getWalletService();
        const author10019Map = await walletService?.fetchMultipleUsers10019(uniqueAuthors) || new Map();
        
        // Initialize all authors as not having 10019
        uniqueAuthors.forEach(author => {
          authorHas10019.set(author, false)
        })
        
        // Process fetched events
        author10019Map.forEach((mintList: any, pubkey: string) => {
          if (!mintList) return; // Skip null entries
          
          // Use wallet service parsing methods instead of manual tag parsing
          const mints = walletService?.parseNutzapMints(mintList) || [];
          const cashuPubkey = walletService?.parseNutzapP2PK(mintList);
          
          if (mints.length > 0 && cashuPubkey) {
            authorMints.set(pubkey, mints)
            authorHas10019.set(pubkey, true)
            authorCashuPubkeys.set(pubkey, cashuPubkey)
          }
          
          // Extract relay list from kind:10019 'relay' tags per NIP-61
          const relayTags = mintList.tags?.filter((tag: string[]) => tag[0] === 'relay') || []
          const nutzapRelays = relayTags.map((tag: string[]) => tag[1]).filter(Boolean)
          
          nutzapRelays.forEach((relay: string) => allNutzapRelays.add(relay))
        })
      }

      // Use the shared NDK instance with outbox model + kind 10019 relay union
      const ndk = (this.props.client as any).ndk
      const uniqueAuthorPubkeys = [...new Set(this.props.group.content?.map(item => item.pubkey) || [])]
      
      console.log('🔍 Fetching nutzaps using outbox model + kind:10019 relays for event authors (recipients):', uniqueAuthorPubkeys)
      console.log('🔍 Filter:', filter)
      
      // First, let NDK discover relays for the event authors (nutzap recipients)
      // by fetching their relay lists, which will populate the outbox tracker
      await Promise.all(uniqueAuthorPubkeys.map(async (pubkey) => {
        try {
          const user = ndk.getUser({ pubkey })
          await user.fetchProfile() // This will trigger relay discovery for this user
        } catch (err) {
          console.debug('Could not fetch profile for relay discovery:', pubkey, err)
        }
      }))
      
      console.log('🔍 Found kind:10019 nutzap relays:', Array.from(allNutzapRelays))
      
      // Fetch nutzaps using true union of:
      // 1. NDK outbox model relays (kind 10002/kind 3) 
      // 2. Kind 10019 nutzap relays (NIP-61 compliance)
      
      let eventsSet = new Set<any>()
      
      // First: Get nutzaps from outbox model (kind 10002/kind 3 relays)
      console.log('🔍 Step 1: Fetching via outbox model')
      const outboxEvents = await ndk.fetchEvents(filter)
      outboxEvents.forEach((event: any) => eventsSet.add(event))
      console.log('🔍 Found', outboxEvents.size, 'nutzaps via outbox model')
      
      // Second: Get nutzaps from kind:10019 relays (NIP-61 compliance)
      if (allNutzapRelays.size > 0) {
        const nutzapRelayUrls = Array.from(allNutzapRelays)
        console.log('🔍 Step 2: Fetching via kind:10019 relays:', nutzapRelayUrls)
        
        const nutzapEvents = await ndk.fetchEvents(filter, { 
          relayUrls: nutzapRelayUrls 
        })
        nutzapEvents.forEach((event: any) => eventsSet.add(event))
        console.log('🔍 Found', nutzapEvents.size, 'nutzaps via kind:10019 relays')
      } else {
        console.log('🔍 Step 2: No kind:10019 relays found, skipping')
      }
      
      console.log('🔍 Total unique nutzaps found:', eventsSet.size)
      const events = Array.from(eventsSet)
      const nutzapTotals = new Map<string, number>()
      const seenEventIds = new Set<string>() // Track event IDs just for this initial fetch

      events.forEach((event: any) => {
        console.log('🔍 Processing nutzap event:', event.id, 'from author:', event.pubkey)
        
        // Check if we've already processed this event ID in this batch
        if (seenEventIds.has(event.id)) {
          console.log('  ⏭️ Skipping duplicate event ID:', event.id)
          return
        }
        seenEventIds.add(event.id)
        // Find the event tag
        const eventTag = event.tags.find((tag: string[]) => tag[0] === 'e')
        if (!eventTag) {
          console.log('  ❌ No event tag found in nutzap')
          return
        }

        const eventId = eventTag[1]
        console.log('  🎯 Nutzap targets event:', eventId)
        
        // Get the author of the target event
        const eventAuthor = eventAuthors.get(eventId)
        if (!eventAuthor) {
          console.log('  ❌ Event ID not found in current group content:', eventId)
          console.log('  📝 Available event IDs:', Array.from(eventAuthors.keys()))
          return
        }
        console.log('  👤 Event author:', eventAuthor)
        
        // Get the authorized mints for this event's author
        const authorizedMints = authorMints.get(eventAuthor) || []
        const cashuPubkey = authorCashuPubkeys.get(eventAuthor)
        
        // Skip if author has no 10019 config
        if (authorizedMints.length === 0 || !cashuPubkey) return
        
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
        
        // Find the proof tag and extract amount + verify P2PK lock
        const proofTag = event.tags.find((tag: string[]) => tag[0] === 'proof')
        if (!proofTag) return

        // Get amount from proof - NIP-61 nutzaps don't have amount tags
        let amount: number
        
        try {
          const proof = JSON.parse(proofTag[1])
          amount = proof.amount
          if (!amount || isNaN(amount)) return
          
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
          // Note: DLEQ proof verification is not implemented as it would require
          // fetching and caching mint keysets, which is complex for the frontend
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

      // Check compatibility for all authors
      this.checkAuthorsCompatibility(uniqueAuthors)

      // Subscribe to new nutzaps using the shared NDK instance with outbox model
      // Add 'since' timestamp to only get events created after this moment
      const subscriptionFilter = {
        ...filter,
        since: Math.floor(Date.now() / 1000) // Current timestamp in seconds
      }
      
      console.log('🔔 Subscribing to nutzaps using outbox model')
      this.nutzapSubscription = await ndk.subscribe(subscriptionFilter, {
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
        
        // Find the mint tag - check 'u' tag as per NIP-61
        const uTag = event.tags.find((tag: string[]) => tag[0] === 'u')
        const mint = uTag ? uTag[1] : null

        // According to NIP-61, nutzaps MUST have a 'u' tag
        if (!mint) return
        
        // Normalize mint URL for comparison (remove trailing slash)
        const normalizedMint = mint.replace(/\/$/, '')
        
        // Verify mint is authorized (authorizedMints are already normalized)
        if (!authorizedMints.includes(normalizedMint)) return
        
        // Find the proof tag and extract amount + verify P2PK lock
        const proofTag = event.tags.find((tag: string[]) => tag[0] === 'proof')
        if (!proofTag) return

        // Get amount from proof - NIP-61 nutzaps don't have amount tags  
        let amount: number
        
        try {
          const proof = JSON.parse(proofTag[1])
          amount = proof.amount
          if (!amount || isNaN(amount)) return
          
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
          // Note: DLEQ proof verification is not implemented
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

  // Check compatibility for all authors in parallel
  private checkAuthorsCompatibility = async (authors: string[]) => {
    try {
      const walletService = this.props.client.getWalletService();
      if (!walletService) return;

      // Check compatibility for all authors in parallel
      const compatibilityPromises = authors.map(async (pubkey) => {
        try {
          const compatibility = await walletService.canSendToRecipient(pubkey, MIN_NUTZAP_AMOUNT);
          return { pubkey, compatibility };
        } catch (error) {
          console.warn(`Failed to check compatibility for ${pubkey}:`, error);
          return { 
            pubkey, 
            compatibility: {
              canSend: false,
              compatibleBalance: 0,
              compatibleMints: [],
              recipientMints: [],
              reason: "Error checking compatibility"
            }
          };
        }
      });

      const results = await Promise.all(compatibilityPromises);
      
      // Update state with compatibility results
      const authorCompatibility = new Map(this.state.authorCompatibility);
      results.forEach(({ pubkey, compatibility }) => {
        authorCompatibility.set(pubkey, {
          canSend: compatibility.canSend,
          compatibleBalance: compatibility.compatibleBalance,
          compatibleMints: compatibility.compatibleMints,
          reason: compatibility.reason
        });
      });

      this.setState({ authorCompatibility });

    } catch (error) {
      console.error('Failed to check authors compatibility:', error);
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
      const balance = await this.props.client.getCashuBalance()
      this.setState({ walletBalance: balance })
    } catch (error) {
      console.error('Failed to fetch wallet balance:', error)
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
    if (sats > walletBalance) {
      this.setState({ nutzapError: `Insufficient balance. You have ${walletBalance} sats but tried to send ${sats} sats.` })
      return
    }

    this.setState({ nutzapLoading: true, nutzapError: null })

    try {
      // No timeout needed - the CashuWalletService now handles this with NDK zapper events
      await this.props.client.sendNutzapToEvent(eventId, sats);
      
      // SUCCESS - Just close the modal and show success message
      // The nutzap total will update when the event arrives via subscription
      this.setState({ 
        showNutzapModal: null, 
        nutzapAmount: '', 
        nutzapError: null
      })
      
      this.props.showMessage('Nutzap sent to event successfully!', 'success')
      if (this.props.onNutzapSent) this.props.onNutzapSent()
    } catch (error) {
      let errorMessage = 'Failed to send nutzap';
      if (error instanceof Error) {
        if (error.message.includes('No zap method available') || error.message.includes('NIP-61 fallback is disabled')) {
          errorMessage = 'This user cannot receive nutzaps. They need to set up their nutzap wallet first.';
        } else {
          errorMessage = error.message;
        }
      }
      this.setState({ 
        nutzapError: errorMessage
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
                        profileData={
                          this.props.group.memberProfiles?.has(item.pubkey) 
                            ? {
                                profile: this.props.group.memberProfiles.get(item.pubkey)?.profile,
                                has10019: this.props.group.memberProfiles.get(item.pubkey)?.has10019
                              }
                            : undefined
                        }
                      />
                      <span>·</span>
                      <span>
                        {this.formatTimestamp(item.created_at)}
                      </span>
                    </div>
                    <p class="text-sm text-[var(--color-text-primary)] break-all whitespace-pre-wrap leading-relaxed mt-0.5">
                      {item.content}
                    </p>
                  </div>

                  <div class="flex items-center gap-1">
                    {/* Event Nutzap Button and Total */}
                    {((hasWalletBalance && 
                      item.pubkey !== this.getCurrentUserPubkey() && 
                      this.state.authorCompatibility.get(item.pubkey)?.canSend) || 
                      this.state.eventNutzaps.get(item.id)) && (
                      <div class="flex items-center gap-1">
                        {hasWalletBalance && 
                         item.pubkey !== this.getCurrentUserPubkey() && (
                          this.state.authorCompatibility.get(item.pubkey)?.canSend ? (
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
                          ) : (
                            <button
                              disabled
                              class="text-[11px] opacity-0 group-hover:opacity-100 text-gray-500 
                                     transition-all duration-150 flex items-center p-1 cursor-not-allowed opacity-50"
                              title={this.state.authorCompatibility.get(item.pubkey)?.reason || "Cannot send nutzap to this user"}
                            >
                              <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                                <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                              </svg>
                            </button>
                          )
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