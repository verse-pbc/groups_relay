import { Component } from 'preact'

interface JoinRequestFormProps {
  groupId: string
  relayUrl: string
}

interface JoinRequestFormState {
  pubkey: string
  inviteCode: string
  isSubmitting: boolean
  error: string | null
}

export class JoinRequestForm extends Component<JoinRequestFormProps, JoinRequestFormState> {
  constructor(props: JoinRequestFormProps) {
    super(props)
    this.state = {
      pubkey: '',
      inviteCode: '',
      isSubmitting: false,
      error: null
    }
  }

  handleSubmit = async (e: Event) => {
    e.preventDefault()
    this.setState({ isSubmitting: true, error: null })

    try {
      // TODO: Implement join request submission
      await new Promise(resolve => setTimeout(resolve, 1000))
    } catch (error) {
      this.setState({ error: 'Failed to submit join request' })
    } finally {
      this.setState({ isSubmitting: false })
    }
  }

  render() {
    const { pubkey, inviteCode, isSubmitting, error } = this.state

    return (
      <form onSubmit={this.handleSubmit} class="space-y-4">
        <input
          type="text"
          value={pubkey}
          onInput={e => this.setState({ pubkey: (e.target as HTMLInputElement).value })}
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
          disabled={isSubmitting || !pubkey.trim()}
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