import { Component } from 'preact'
import { NostrClient, NostrGroupError } from '../api/nostr_client'
import type { Group } from '../types'

interface InviteSectionProps {
  group: Group
  client: NostrClient
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onInviteDelete: (code: string) => void
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

  private showError = (prefix: string, error: unknown) => {
    console.error(prefix, error)
    const message = error instanceof NostrGroupError ? error.displayMessage : String(error)
    this.props.showMessage(`${prefix}: ${message}`, 'error')
  }

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }
  }

  handleCreateInvite = async (e: Event) => {
    e.preventDefault()
    if (!this.state.inviteCode.trim()) return

    this.setState({ isCreatingInvite: true })
    try {
      const inviteEvent = await this.props.client.createInvite(this.props.group.id, this.state.inviteCode)
      
      // Update the group's invites map
      this.props.updateGroupsMap(groupsMap => {
        const group = groupsMap.get(this.props.group.id)
        if (group) {
          const updatedGroup = {
            ...group,
            invites: {
              ...group.invites,
              [this.state.inviteCode]: {
                code: this.state.inviteCode,
                pubkey: inviteEvent.pubkey,
                roles: ['member'], // Default role from the createInvite method
                id: inviteEvent.id
              }
            }
          }
          groupsMap.set(this.props.group.id, updatedGroup)
        }
      })
      
      this.setState({ inviteCode: '' })
      this.props.showMessage('Invite created successfully', 'success')
    } catch (error) {
      this.showError('Failed to create invite', error)
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
        if (group) {
          const updatedGroup = {
            ...group,
            invites: { ...group.invites }
          }
          delete updatedGroup.invites[code]
          groupsMap.set(group.id, updatedGroup)
        }
      })
      this.props.onInviteDelete(code)
      this.props.showMessage('Invite deleted successfully', 'success')
    } catch (error) {
      this.showError('Failed to delete invite', error)
    } finally {
      this.setState({ inviteAction: null })
    }
  }

  handleCopyInviteCode = (code: string) => {
    navigator.clipboard.writeText(code)
    this.props.showMessage('Invite code copied to clipboard', 'success')
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
              {Object.entries(invites).map(([code]) => (
                <li key={code} class="group p-3 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]
                                   hover:border-[var(--color-border-hover)] transition-all duration-150">
                  <div class="flex items-center justify-between gap-2">
                    <div class="flex items-center gap-2">
                      <span class="text-sm font-mono text-[var(--color-text-primary)]">
                        {code}
                      </span>
                    </div>
                    <div class="flex items-center gap-2">
                      <button
                        onClick={() => this.copyInviteLink(code)}
                        class="opacity-0 group-hover:opacity-100 text-[11px] text-[var(--color-text-tertiary)]
                               hover:text-[var(--color-text-secondary)] transition-all duration-150 flex items-center gap-1"
                      >
                        {showCopied ? (
                          <>
                            <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <path d="M20 6L9 17L4 12" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                            </svg>
                            Copied!
                          </>
                        ) : (
                          <>
                            <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                              <path d="M8 4v12a2 2 0 002 2h8a2 2 0 002-2V7.242a2 2 0 00-.602-1.43L16.083 2.57A2 2 0 0014.685 2H10a2 2 0 00-2 2z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                              <path d="M16 18v2a2 2 0 01-2 2H6a2 2 0 01-2-2V9a2 2 0 012-2h2" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                            </svg>
                            Copy Join Link
                          </>
                        )}
                      </button>
                      <button
                        onClick={() => {
                          if (inviteAction?.type === 'delete' && inviteAction.code === code) {
                            this.handleDeleteInvite(code)
                          } else {
                            this.setState({ inviteAction: { type: 'delete', code } })
                          }
                        }}
                        class={`opacity-0 group-hover:opacity-100 text-[11px] transition-all duration-150 flex items-center gap-1
                               ${inviteAction?.type === 'delete' && inviteAction.code === code
                                 ? 'text-red-400 hover:text-red-300'
                                 : 'text-red-400 hover:text-red-300'}`}
                        title="Delete invite"
                      >
                        {inviteAction?.type === 'delete' && inviteAction.code === code ? (
                          'Confirm'
                        ) : (
                          <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                            <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                            <path d="M10 11v6M14 11v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                          </svg>
                        )}
                      </button>
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