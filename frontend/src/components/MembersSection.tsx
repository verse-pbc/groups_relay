import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { PubkeyDisplay } from './PubkeyDisplay'

interface MembersSectionProps {
  group: Group
  client: NostrClient
}

interface MembersSectionState {
  newMemberPubkey: string
  isAddingMember: boolean
  removingMembers: Set<string>
  showConfirmRemove: string | null
}

export class MembersSection extends Component<MembersSectionProps, MembersSectionState> {
  state = {
    newMemberPubkey: '',
    isAddingMember: false,
    removingMembers: new Set<string>(),
    showConfirmRemove: null
  }

  handleAddMember = async (e: Event) => {
    e.preventDefault()
    if (!this.state.newMemberPubkey.trim()) return

    this.setState({ isAddingMember: true })
    try {
      await this.props.client.addMember(this.props.group.id, this.state.newMemberPubkey)
      this.setState({ newMemberPubkey: '' })
    } catch (error) {
      console.error('Failed to add member:', error)
    } finally {
      this.setState({ isAddingMember: false })
    }
  }

  handleRemoveMember = async (pubkey: string) => {
    this.setState(prev => ({
      removingMembers: new Set(prev.removingMembers).add(pubkey),
      showConfirmRemove: null
    }))

    try {
      await this.props.client.removeMember(this.props.group.id, pubkey)
    } catch (error) {
      console.error('Failed to remove member:', error)
    } finally {
      this.setState(prev => {
        const newSet = new Set(prev.removingMembers)
        newSet.delete(pubkey)
        return { removingMembers: newSet }
      })
    }
  }

  formatRole(role: string): string {
    return role.charAt(0).toUpperCase() + role.slice(1).toLowerCase()
  }

  render() {
    const { group } = this.props
    const { newMemberPubkey, isAddingMember, removingMembers, showConfirmRemove } = this.state

    return (
      <div class="space-y-4">
        <div class="p-4 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
          <form onSubmit={this.handleAddMember}>
            <div class="flex gap-2">
              <input
                type="text"
                value={newMemberPubkey}
                onChange={(e) => this.setState({ newMemberPubkey: (e.target as HTMLInputElement).value })}
                placeholder="Enter member pubkey"
                class="flex-1 px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                       text-sm rounded-lg text-[var(--color-text-primary)]
                       placeholder-[var(--color-text-tertiary)]
                       focus:outline-none focus:ring-1 focus:ring-accent
                       hover:border-[var(--color-border-hover)] transition-colors"
                disabled={isAddingMember}
              />
              <button
                type="submit"
                disabled={isAddingMember || !newMemberPubkey.trim()}
                class="shrink-0 px-3 py-1.5 bg-accent text-white rounded-lg text-sm font-medium
                       hover:bg-accent/90 disabled:opacity-50 disabled:cursor-not-allowed
                       transition-colors flex items-center justify-center w-[70px]"
              >
                {isAddingMember ? (
                  <>
                    <span class="animate-spin">⚡</span>
                  </>
                ) : (
                  'Add'
                )}
              </button>
            </div>
          </form>
        </div>

        <div class="space-y-2">
          {group.members.map(member => (
            <div
              key={member.pubkey}
              class="flex items-center gap-2 p-3 bg-[var(--color-bg-primary)]
                     rounded-lg border border-[var(--color-border)] hover:border-[var(--color-border-hover)]
                     transition-colors"
            >
              <div class="flex items-center gap-2 min-w-0 flex-1 overflow-hidden">
                <PubkeyDisplay pubkey={member.pubkey} showCopy={false} />
                {member.roles.map(role => (
                  <span
                    key={role}
                    class="shrink-0 px-1.5 py-0.5 text-xs font-medium bg-[var(--color-bg-secondary)]
                           text-[var(--color-text-secondary)] rounded"
                  >
                    {this.formatRole(role)}
                  </span>
                ))}
              </div>

              {showConfirmRemove === member.pubkey ? (
                <div class="flex items-center gap-1 shrink-0">
                  <button
                    onClick={() => this.handleRemoveMember(member.pubkey)}
                    class="px-2 py-1 text-xs text-red-400 hover:text-red-300 transition-colors"
                  >
                    Confirm
                  </button>
                  <button
                    onClick={() => this.setState({ showConfirmRemove: null })}
                    class="px-2 py-1 text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <button
                  onClick={() => this.setState({ showConfirmRemove: member.pubkey })}
                  disabled={removingMembers.has(member.pubkey)}
                  class="shrink-0 px-2 py-1 text-xs text-[var(--color-text-tertiary)] hover:text-red-400 transition-colors
                         disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-1"
                >
                  {removingMembers.has(member.pubkey) ? (
                    <>
                      <span class="animate-spin">⚡</span>
                      Remove
                    </>
                  ) : (
                    'Remove'
                  )}
                </button>
              )}
            </div>
          ))}

          {group.members.length === 0 && (
            <div class="text-center py-12">
              <div class="mb-3 text-2xl">👥</div>
              <p class="text-sm text-[var(--color-text-tertiary)]">No members yet</p>
              <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
                Add members using their public key
              </p>
            </div>
          )}
        </div>
      </div>
    )
  }
}