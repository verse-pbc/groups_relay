import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import { nip19 } from 'nostr-tools'

interface JoinRequestFormProps {
  groupId: string
  relayUrl: string
  client: NostrClient
}

interface JoinRequestFormState {
  sec: string
  inviteCode: string
  isSubmitting: boolean
  error: string | null
}

export class JoinRequestForm extends Component<JoinRequestFormProps, JoinRequestFormState> {
  constructor(props: JoinRequestFormProps) {
    super(props)
    this.state = {
      sec: '',
      inviteCode: '',
      isSubmitting: false,
      error: null
    }
  }

  handleSubmit = async (e: Event) => {
    e.preventDefault()
    if (!this.state.sec.trim()) return

    let hexKey = this.state.sec
    if (this.state.sec.startsWith('nsec')) {
      try {
        const { data: decodedKey } = nip19.decode(this.state.sec)
        hexKey = decodedKey as string
      } catch (e) {
        this.setState({ error: 'Invalid nsec format. Please check your private key.' })
        return
      }
    }

    this.setState({ error: '', isSubmitting: true })
    try {
      // Create a temporary client with the provided hex key so we can sign the join request
      const tempClient = new NostrClient(hexKey, { relayUrl: this.props.relayUrl })

      try {
        await tempClient.connect()
      } catch (err) {
        const error = err as Error
        throw new Error(`Failed to connect to relay: ${error?.message || 'Unknown error'}`)
      }

      console.log('connected')
      try {
        await tempClient.sendJoinRequest(this.props.groupId, this.state.inviteCode || undefined)

        this.setState({
          sec: '',
          inviteCode: ''
        })
      } catch (err) {
        const error = err as { name?: string; message?: string }
        if (error?.name === 'NostrGroupError') {
          throw new Error(error.message || 'Unknown group error')
        }
        throw new Error(`Failed to send join request: ${error?.message || 'Unknown error'}`)
      }
    } catch (err) {
      const error = err as Error
      console.error('Join request error:', error)
      this.setState({
        error: error?.message || 'Failed to send join request. Please check your nsec and try again.'
      })
    } finally {
      this.setState({ isSubmitting: false })
    }
  }

  render() {
    const { sec, inviteCode, isSubmitting, error } = this.state

    return (
      <form onSubmit={this.handleSubmit} class="space-y-4">
        <input
          type="text"
          value={sec}
          onInput={e => this.setState({ sec: (e.target as HTMLInputElement).value })}
          placeholder="Your nsec or hex key"
          class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                 text-sm rounded-lg text-[var(--color-text-primary)]
                 placeholder-[var(--color-text-tertiary)]
                 focus:outline-none focus:ring-1 focus:ring-accent
                 hover:border-[var(--color-border-hover)] transition-colors"
          required
          disabled={isSubmitting}
        />

        <input
          type="text"
          value={inviteCode}
          onInput={e => this.setState({ inviteCode: (e.target as HTMLInputElement).value })}
          placeholder="Invite Code (optional)"
          class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                 text-sm rounded-lg text-[var(--color-text-primary)]
                 placeholder-[var(--color-text-tertiary)]
                 focus:outline-none focus:ring-1 focus:ring-accent
                 hover:border-[var(--color-border-hover)] transition-colors"
          disabled={isSubmitting}
        />

        <button
          type="submit"
          disabled={isSubmitting || !sec.trim()}
          class="w-full px-4 py-2 bg-accent text-white rounded-lg text-sm font-medium
                 hover:bg-accent/90 disabled:opacity-50 disabled:cursor-not-allowed
                 transition-colors flex items-center justify-center gap-2"
        >
          {isSubmitting ? (
            <>
              <span class="animate-spin">⚡</span>
              Joining...
            </>
          ) : (
            'Join Group'
          )}
        </button>

        {error && (
          <div class="mt-2 text-xs text-red-400">
            {error}
          </div>
        )}
      </form>
    )
  }
}