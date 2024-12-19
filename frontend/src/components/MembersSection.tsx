import { Component } from 'preact'
import type { Group } from '../types'

interface MembersSectionProps {
  group: Group
  newMemberPubkey: string
  isAddingMember: boolean
  onMemberPubkeyChange: (pubkey: string) => void
  onAddMember: (e: Event) => void
  onRemoveMember: (pubkey: string) => void
}

export class MembersSection extends Component<MembersSectionProps> {
  truncatePubkey = (pubkey: string) => {
    return pubkey.slice(0, 8) + '...'
  }

  render() {
    const { group, newMemberPubkey, isAddingMember, onMemberPubkeyChange, onAddMember, onRemoveMember } = this.props

    return (
      <div class="p-3">
        <h3 class="flex items-center gap-1 text-sm font-semibold text-[var(--color-text-primary)] mb-2">
          <span class="text-base">👥</span> Members
        </h3>

        <form onSubmit={onAddMember} class="mb-3">
          <div class="flex gap-2">
            <input
              type="text"
              value={newMemberPubkey}
              onInput={e => onMemberPubkeyChange((e.target as HTMLInputElement).value)}
              placeholder="Enter member pubkey"
              class="flex-1 rounded border border-[var(--color-border)] px-2 py-1 text-xs
                     bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                     focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                     focus:ring-[var(--color-accent)]/10 transition-all font-mono"
              required
              disabled={isAddingMember}
            />
            <button
              type="submit"
              disabled={isAddingMember || !newMemberPubkey.trim()}
              class="px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                     hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                     transition-all flex items-center gap-1 disabled:opacity-50 whitespace-nowrap"
            >
              {isAddingMember ? (
                <>
                  <span class="animate-spin">⌛</span>
                  Adding...
                </>
              ) : (
                'Add Member'
              )}
            </button>
          </div>
        </form>

        <ul class="space-y-2 max-h-[300px] overflow-y-auto">
          {group.members.map(member => (
            <li key={member.pubkey} class="py-1">
              <div class="flex items-center justify-between gap-2">
                <div
                  class="text-xs text-[var(--color-text-secondary)] font-mono hover:text-[var(--color-text-primary)] transition-colors"
                  data-tooltip={member.pubkey}
                >
                  {this.truncatePubkey(member.pubkey)}
                </div>
                <div class="flex items-center gap-2">
                  <div class="flex flex-wrap gap-1 flex-shrink-0">
                    {member.roles.map(role => {
                      const lower = role.toLowerCase()
                      const [icon] = lower.includes("admin")
                        ? ["⭐"]
                        : lower.includes("moderator")
                          ? [""]
                          : ["👤"]
                      return (
                        <span class={`role-badge ${lower.includes("admin") ? "admin" : lower.includes("moderator") ? "moderator" : "member"} text-xs`}>
                          {icon} {role}
                        </span>
                      )
                    })}
                  </div>
                  <button
                    onClick={() => onRemoveMember(member.pubkey)}
                    class="text-red-400 hover:text-red-300 transition-colors flex-shrink-0 p-1"
                    title="Remove member"
                  >
                    ❌
                  </button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      </div>
    )
  }
}