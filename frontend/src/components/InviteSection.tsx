import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'

interface InviteSectionProps {
  group: Group
  client: NostrClient
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
}

interface InviteSectionState {
  isCreatingInvite: boolean
  inviteCode: string
  error: string
  showCopied: boolean
  inviteAction: { type: 'delete', code: string } | null
}

export class InviteSection extends Component<InviteSectionProps, InviteSectionState> {
  private instanceId: string;
  private copyTimeout: number | null = null;

  constructor(props: InviteSectionProps) {
    super(props)
    this.instanceId = Math.random().toString(36).substring(2, 9);
    this.state = {
      isCreatingInvite: false,
      inviteCode: '',
      error: '',
      showCopied: false,
      inviteAction: null
    }
  }

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
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

  generateRandomCode = () => {
    const array = new Uint8Array(12)
    crypto.getRandomValues(array)
    const code = Array.from(array, byte => byte.toString(16).padStart(2, '0')).join('')
    this.setState({ inviteCode: code })
  }

  copyInviteLink = (code: string) => {
    const link = `plur://join-community?group-id=${this.props.group.id}&code=${code}`
    navigator.clipboard.writeText(link)
    this.setState({ showCopied: true })

    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }

    this.copyTimeout = window.setTimeout(() => {
      this.setState({ showCopied: false })
    }, 2000)
  }

  handleDeleteInvite = async (code: string) => {
    const invite = this.props.group.invites[code]
    if (!invite?.id) return

    this.setState({ inviteAction: { type: 'delete', code } })

    try {
      await this.props.client.deleteEvent(this.props.group.id, invite.id)
      this.props.updateGroupsMap(groupsMap => {
        const group = groupsMap.get(this.props.group.id)
        if (group?.invites) {
          delete group.invites[code]
        }
      })
    } catch (error) {
      console.error('Failed to delete invite:', error)
    } finally {
      this.setState({ inviteAction: null })
    }
  }

  render() {
    const { group } = this.props
    const { isCreatingInvite, inviteCode, error, showCopied, inviteAction } = this.state
    const invites = group.invites || {}
    const hasInvites = Object.keys(invites).length > 0

    return (
      <div class="space-y-4">
        <div class="p-4 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
          <form onSubmit={this.handleCreateInvite} class="space-y-3">
            <div class="flex items-center gap-2">
              <div class="flex-1">
                <div class="relative">
                  <input
                    type="text"
                    id={`create-invite-code-${this.instanceId}`}
                    value={inviteCode}
                    onInput={e => this.setState({ inviteCode: (e.target as HTMLInputElement).value })}
                    placeholder="Enter invite code"
                    maxLength={32}
                    class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                           text-sm rounded-lg text-[var(--color-text-primary)] font-mono
                           placeholder-[var(--color-text-tertiary)]
                           focus:outline-none focus:ring-1 focus:ring-accent
                           hover:border-[var(--color-border-hover)] transition-colors"
                    required
                    disabled={isCreatingInvite}
                  />
                </div>
              </div>
              <button
                type="button"
                onClick={this.generateRandomCode}
                class="shrink-0 px-3 py-2 text-sm text-[var(--color-text-tertiary)]
                       hover:text-[var(--color-text-secondary)] transition-colors"
                title="Generate random code"
              >
                🎲
              </button>
              <button
                type="submit"
                disabled={isCreatingInvite || !inviteCode.trim()}
                class="shrink-0 px-4 py-2 bg-accent text-white rounded-lg text-sm font-medium
                       hover:bg-accent/90 disabled:opacity-50 disabled:cursor-not-allowed
                       transition-colors flex items-center justify-center min-w-[80px]"
              >
                {isCreatingInvite ? (
                  <span class="animate-spin">⚡</span>
                ) : (
                  'Create'
                )}
              </button>
            </div>
            {error && (
              <div class="text-xs text-red-400">
                {error}
              </div>
            )}
          </form>
        </div>

        <div class="space-y-2">
          {hasInvites ? (
            <ul class="space-y-2">
              {Object.entries(invites).map(([code, invite]) => (
                <li key={code} class="group p-3 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]
                                   hover:border-[var(--color-border-hover)] transition-all duration-150">
                  <div class="space-y-2">
                    <div class="flex items-center justify-between gap-2">
                      <div class="flex items-center gap-2">
                        <span class="text-sm font-mono text-[var(--color-text-primary)]">
                          {code}
                        </span>
                        {invite.pubkey && (
                          <span class="text-xs text-[var(--color-text-tertiary)]">
                            • Used
                          </span>
                        )}
                      </div>
                      <div class="flex items-center gap-1">
                        <button
                          onClick={() => this.copyInviteLink(code)}
                          class="opacity-0 group-hover:opacity-100 text-xs text-[var(--color-text-tertiary)]
                                 hover:text-[var(--color-text-secondary)] transition-all"
                        >
                          {showCopied ? 'Copied!' : 'Copy Join Link'}
                        </button>
                        <button
                          onClick={() => {
                            if (inviteAction?.type === 'delete' && inviteAction.code === code) {
                              this.handleDeleteInvite(code)
                            } else {
                              this.setState({ inviteAction: { type: 'delete', code } })
                            }
                          }}
                          class={`text-[11px] opacity-0 group-hover:opacity-100 transition-all duration-150
                                 ${inviteAction?.type === 'delete' && inviteAction.code === code
                                   ? 'text-red-400 hover:text-red-300'
                                   : 'text-[var(--color-text-tertiary)] hover:text-red-400'}`}
                        >
                          {inviteAction?.type === 'delete' && inviteAction.code === code ? (
                            'Delete?'
                          ) : (
                            '×'
                          )}
                        </button>
                      </div>
                    </div>
                  </div>
                </li>
              ))}
            </ul>
          ) : (
            <div class="text-center py-12">
              <div class="mb-3 text-2xl">🎟️</div>
              <p class="text-sm text-[var(--color-text-tertiary)]">No invites created yet</p>
              <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
                Create an invite code to let others join this group
              </p>
              <button
                onClick={this.generateRandomCode}
                class="mt-4 px-4 py-2 bg-[var(--color-bg-secondary)] text-[var(--color-text-secondary)]
                       rounded-lg text-sm hover:text-[var(--color-text-primary)] transition-colors
                       flex items-center gap-2 mx-auto"
              >
                <span>🎲</span>
                Generate Random Code
              </button>
            </div>
          )}
        </div>
      </div>
    )
  }
}