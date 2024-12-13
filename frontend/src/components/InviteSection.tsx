import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'

interface InviteSectionProps {
  group: Group
  client: NostrClient
}

interface InviteSectionState {
  isCreatingInvite: boolean
  inviteCode: string
  error: string
}

export class InviteSection extends Component<InviteSectionProps, InviteSectionState> {
  constructor(props: InviteSectionProps) {
    super(props)
    this.state = {
      isCreatingInvite: false,
      inviteCode: '',
      error: ''
    }
  }

  handleCreateInvite = async (e: Event) => {
    e.preventDefault()
    if (!this.state.inviteCode.trim()) return

    this.setState({ error: '', isCreatingInvite: true })
    try {
      await this.props.client.createInvite(this.props.group.id, this.state.inviteCode)
      this.setState({ inviteCode: '' })
    } catch (error) {
      console.error('Failed to create invite:', error)
      this.setState({ error: 'Failed to create invite. Please try again.' })
    } finally {
      this.setState({ isCreatingInvite: false })
    }
  }

  render() {
    const { group } = this.props
    const { isCreatingInvite, inviteCode, error } = this.state

    const truncatePubkey = (pubkey: string) => {
      return pubkey.slice(0, 8) + '...'
    }

    return (
      <section class="border-t border-[var(--color-border)] p-3">
        <h3 class="flex items-center gap-1 text-sm font-semibold text-[var(--color-text-primary)] mb-2">
          <span class="text-base">🎟️</span> Invites
        </h3>

        <form onSubmit={this.handleCreateInvite} class="mb-3">
          <div class="flex gap-2">
            <input
              type="text"
              value={inviteCode}
              onInput={e => this.setState({ inviteCode: (e.target as HTMLInputElement).value })}
              placeholder="Enter invite code"
              class="flex-1 rounded border border-[var(--color-border)] px-2 py-1 text-xs
                     bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                     focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                     focus:ring-[var(--color-accent)]/10 transition-all"
              required
              disabled={isCreatingInvite}
            />
            <button
              type="submit"
              disabled={isCreatingInvite || !inviteCode.trim()}
              class="px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                     hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                     transition-all flex items-center gap-1 disabled:opacity-50 whitespace-nowrap"
            >
              {isCreatingInvite ? (
                <>
                  <span class="animate-spin">⌛</span>
                  Creating...
                </>
              ) : (
                'Create Invite'
              )}
            </button>
          </div>
          {error && (
            <div class="mt-1 text-xs text-red-400">
              {error}
            </div>
          )}
        </form>

        {group.invites && Object.entries(group.invites).length > 0 ? (
          <ul class="space-y-2">
            {Object.entries(group.invites).map(([code, invite]) => (
              <li key={code} class="py-1">
                <div class="space-y-2">
                  <div class="flex items-center justify-between gap-2">
                    <div class="text-xs text-[var(--color-text-secondary)] font-mono">
                      Code: {code}
                    </div>
                  </div>
                  {invite.pubkey && (
                    <div class="flex items-center justify-between gap-2">
                      <div
                        class="text-xs text-[var(--color-text-secondary)] font-mono hover:text-[var(--color-text-primary)] transition-colors"
                        data-tooltip={invite.pubkey}
                      >
                        Used by: {truncatePubkey(invite.pubkey)}
                      </div>
                    </div>
                  )}
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <p class="text-xs text-[var(--color-text-secondary)]">No invites created yet.</p>
        )}
      </section>
    )
  }
}