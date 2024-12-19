import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import { nip19 } from 'nostr-tools'

interface JoinRequestFormProps {
  groupId: string
  relayUrl: string
}

interface JoinRequestFormState {
  sec: string
  inviteCode: string
  isSubmitting: boolean
  error: string
}

export class JoinRequestForm extends Component<JoinRequestFormProps, JoinRequestFormState> {
  private instanceId: string;

  constructor(props: JoinRequestFormProps) {
    super(props)
    this.instanceId = Math.random().toString(36).substring(2, 9);
    this.state = {
      sec: '',
      inviteCode: '',
      isSubmitting: false,
      error: ''
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
      <form onSubmit={this.handleSubmit} class="space-y-2">
        <div>
          <label htmlFor={`join-nsec-${this.instanceId}`} class="block text-xs font-medium text-[var(--color-text-secondary)] mb-0.5">
            Your nsec or hex key
          </label>
          <input
            type="password"
            id={`join-nsec-${this.instanceId}`}
            value={sec}
            onInput={e => this.setState({ sec: (e.target as HTMLInputElement).value })}
            placeholder="nsec1..."
            class="block w-full rounded border border-[var(--color-border)] px-2 py-1 text-xs
                   bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                   focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                   focus:ring-[var(--color-accent)]/10 transition-all"
            required
            disabled={isSubmitting}
          />
        </div>

        <div>
          <label htmlFor={`join-invite-code-${this.instanceId}`} class="block text-xs font-medium text-[var(--color-text-secondary)] mb-0.5">
            Invite Code (optional)
          </label>
          <input
            type="text"
            id={`join-invite-code-${this.instanceId}`}
            value={inviteCode}
            onInput={e => this.setState({ inviteCode: (e.target as HTMLInputElement).value })}
            placeholder="Enter invite code"
            class="block w-full rounded border border-[var(--color-border)] px-2 py-1 text-xs
                   bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                   focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                   focus:ring-[var(--color-accent)]/10 transition-all"
            disabled={isSubmitting}
          />
        </div>

        <button
          type="submit"
          disabled={isSubmitting || !sec.trim()}
          class="w-full px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                 hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                 transition-all flex items-center justify-center gap-1 disabled:opacity-50"
        >
          {isSubmitting ? (
            <>
              <span class="animate-spin">⌛</span>
              Joining...
            </>
          ) : (
            'Join Group'
          )}
        </button>

        {error && (
          <div class="text-xs text-red-400">
            {error}
          </div>
        )}
      </form>
    )
  }
}