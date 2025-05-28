import { Component } from 'preact'
import { NostrClient, NostrGroupError } from '../api/nostr_client'
import type { Group } from '../types'
import { Member } from './Member'

interface MembersSectionProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  isAdmin?: boolean
}

interface MembersSectionState {
  newMemberNpub: string
  isAddingMember: boolean
  removingMembers: Set<string>
  showConfirmRemove: string | null
}

export class MembersSection extends Component<MembersSectionProps, MembersSectionState> {
  state = {
    newMemberNpub: '',
    isAddingMember: false,
    removingMembers: new Set<string>(),
    showConfirmRemove: null
  }

  private showError = (prefix: string, error: unknown) => {
    console.error(prefix, error)
    const message = error instanceof NostrGroupError ? error.displayMessage : String(error)
    this.props.showMessage(`${prefix}: ${message}`, 'error')
  }

  handleAddMember = async (e: Event) => {
    e.preventDefault()
    if (!this.state.newMemberNpub.trim()) return

    this.setState({ isAddingMember: true })
    try {
      const input = this.state.newMemberNpub.trim()
      let pubkey: string

      if (input.includes('@')) {
        // Handle NIP-05
        pubkey = await this.props.client.resolveNip05(input)
      } else {
        // Handle npub
        pubkey = this.props.client.npubToPubkey(input)
      }

      await this.props.client.addMember(this.props.group.id, pubkey)
      this.setState({ newMemberNpub: '' })
      this.props.showMessage('Member added successfully!', 'success')
    } catch (error) {
      this.showError('Failed to add member', error)
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
      this.props.group.members = this.props.group.members.filter(m => m.pubkey !== pubkey)
      this.props.showMessage('Member removed successfully', 'success')
    } catch (error) {
      this.showError('Failed to remove member', error)
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
    const { group, client, showMessage, isAdmin } = this.props
    const { newMemberNpub, isAddingMember, removingMembers, showConfirmRemove } = this.state

    // Get wallet state from client
    const cashuProofs = client.getCashuProofs()
    const mints = client.getWalletMints()

    return (
      <div class="space-y-4">
        {isAdmin && (
          <div class="p-4 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
            <form onSubmit={this.handleAddMember}>
            <div class="flex gap-2">
              <input
                type="text"
                value={newMemberNpub}
                onChange={(e) => this.setState({ newMemberNpub: (e.target as HTMLInputElement).value })}
                placeholder="Enter member npub or name@domain.com"
                class="flex-1 px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                       text-sm rounded-lg text-[var(--color-text-primary)]
                       placeholder-[var(--color-text-tertiary)]
                       focus:outline-none focus:ring-1 focus:ring-accent
                       hover:border-[var(--color-border-hover)] transition-colors"
                disabled={isAddingMember}
              />
              <button
                type="submit"
                disabled={!newMemberNpub.trim()}
                class="shrink-0 px-4 py-2 bg-accent text-white rounded-lg text-sm font-medium
                       hover:bg-accent/90 disabled:opacity-50 disabled:cursor-not-allowed
                       transition-colors flex items-center justify-center w-[70px]"
              >
                {isAddingMember ? (
                  <span class="animate-spin">⚡</span>
                ) : (
                  'Add'
                )}
              </button>
            </div>
          </form>
          </div>
        )}

        <div class="space-y-2">
          {group.members.map(member => (
            <Member
              key={member.pubkey}
              member={member}
              group={group}
              client={client}
              showMessage={showMessage}
              onRemove={this.handleRemoveMember}
              isRemoving={removingMembers.has(member.pubkey)}
              showConfirmRemove={showConfirmRemove === member.pubkey}
              onShowConfirmRemove={() => this.setState({ showConfirmRemove: member.pubkey })}
              onHideConfirmRemove={() => this.setState({ showConfirmRemove: null })}
              cashuProofs={cashuProofs}
              mints={mints}
              onNutzapSent={() => {
                // Trigger a refresh or update if needed
              }}
            />
          ))}

          {group.members.length === 0 && (
            <div class="text-center py-12">
              <div class="mb-3 text-2xl">👥</div>
              <p class="text-sm text-[var(--color-text-tertiary)]">No members yet</p>
              <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
                Add members using their npub or NIP-05 address
              </p>
            </div>
          )}
        </div>
      </div>
    )
  }
}