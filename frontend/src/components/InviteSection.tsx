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
  private instanceId: string;

  constructor(props: InviteSectionProps) {
    super(props)
    this.instanceId = Math.random().toString(36).substring(2, 9);
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
      <div class="space-y-4">
        <div class="p-4 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
          <form onSubmit={this.handleCreateInvite}>
            <div class="flex gap-2">
              <input
                type="text"
                id={`create-invite-code-${this.instanceId}`}
                value={inviteCode}
                onInput={e => this.setState({ inviteCode: (e.target as HTMLInputElement).value })}
                placeholder="Enter invite code"
                class="min-w-0 flex-1 px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                       text-sm rounded-lg text-[var(--color-text-primary)]
                       placeholder-[var(--color-text-tertiary)]
                       focus:outline-none focus:ring-1 focus:ring-accent
                       hover:border-[var(--color-border-hover)] transition-colors"
                required
                disabled={isCreatingInvite}
              />
              <button
                type="submit"
                disabled={isCreatingInvite || !inviteCode.trim()}
                class="shrink-0 px-3 py-1.5 bg-accent text-white rounded-lg text-sm font-medium
                       hover:bg-accent/90 disabled:opacity-50 disabled:cursor-not-allowed
                       transition-colors flex items-center justify-center w-[80px]"
              >
                {isCreatingInvite ? (
                  <span class="animate-spin">⚡</span>
                ) : (
                  'Create'
                )}
              </button>
            </div>
            {error && (
              <div class="mt-2 text-xs text-red-400">
                {error}
              </div>
            )}
          </form>
        </div>

        <div class="space-y-2">
          {group.invites && Object.entries(group.invites).length > 0 ? (
            <ul class="space-y-2">
              {Object.entries(group.invites).map(([code, invite]) => (
                <li key={code} class="p-2.5 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
                  <div class="space-y-2">
                    <div class="flex items-center justify-between gap-2">
                      <div class="text-xs font-mono text-[var(--color-text-primary)]">
                        Code: {code}
                      </div>
                      <a
                        href={`plur://join-community?group-id=${group.id}&code=${code}`}
                        class="text-xs text-accent hover:text-accent/90 transition-colors"
                      >
                        Join Link
                      </a>
                    </div>
                    {invite.pubkey && (
                      <div class="flex items-center justify-between gap-2">
                        <div
                          class="text-xs font-mono text-[var(--color-text-secondary)]"
                          title={invite.pubkey}
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
            <div class="text-center py-12">
              <div class="mb-3 text-2xl">🎟️</div>
              <p class="text-sm text-[var(--color-text-tertiary)]">No invites created yet</p>
              <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
                Create an invite code to let others join
              </p>
            </div>
          )}
        </div>
      </div>
    )
  }
}