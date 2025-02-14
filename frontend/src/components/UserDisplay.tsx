import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'

interface UserDisplayProps {
  pubkey: string
  showCopy?: boolean
  client?: NostrClient
  size?: 'sm' | 'md' | 'lg'
  onCopy?: () => void
  isRelayAdmin?: boolean
}

interface UserDisplayState {
  profilePicture: string | null
  displayId: string
  displayName: string | null
  copied: boolean
}

export class UserDisplay extends Component<UserDisplayProps, UserDisplayState> {
  private copyTimeout: number | null = null;

  state = {
    profilePicture: null,
    displayId: '',
    displayName: null,
    copied: false
  }

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }
  }

  async componentDidMount() {
    const { pubkey, client } = this.props

    // Convert to npub if it's a hex pubkey
    const displayId = pubkey.startsWith('npub') ? pubkey : client?.pubkeyToNpub(pubkey) || pubkey
    this.setState({ displayId })

    // Fetch profile if client is provided
    if (client) {
      // If input is npub, convert to hex for profile fetch
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
    }, 2000)
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
    const { pubkey, showCopy = true, isRelayAdmin = false } = this.props
    const { profilePicture, displayId, displayName, copied } = this.state
    const sizeClasses = this.getSizeClasses()

    return (
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
    )
  }
}