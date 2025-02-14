import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { UserDisplay } from './UserDisplay'

interface MemberProps {
  member: {
    pubkey: string
    roles: string[]
  }
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onRemove: (pubkey: string) => Promise<void>
  isRemoving: boolean
  showConfirmRemove: boolean
  onShowConfirmRemove: () => void
  onHideConfirmRemove: () => void
}

interface MemberState {
  npub: string
}

export class Member extends Component<MemberProps, MemberState> {
  state = {
    npub: this.props.client.pubkeyToNpub(this.props.member.pubkey)
  }

  formatRole(role: string): string {
    return role.charAt(0).toUpperCase() + role.slice(1).toLowerCase()
  }

  render() {
    const {
      member,
      group,
      client,
      showMessage,
      onRemove,
      isRemoving,
      showConfirmRemove,
      onShowConfirmRemove,
      onHideConfirmRemove
    } = this.props
    const { npub } = this.state

    return (
      <div
        class="group flex items-center gap-2 p-3 bg-[var(--color-bg-primary)]
               rounded-lg border border-[var(--color-border)] hover:border-[var(--color-border-hover)]
               hover:shadow-sm transition-all duration-150"
      >
        <div class="flex items-center gap-2 min-w-0 flex-1 overflow-hidden">
          <div class="flex items-center gap-2">
            <UserDisplay
              pubkey={npub}
              client={client}
              showCopy={true}
              onCopy={() => showMessage('Npub copied to clipboard', 'success')}
            />
          </div>
          {member.roles.map(role => (
            <span
              key={role}
              class={`shrink-0 px-2 py-1 text-xs font-medium rounded-full
                      ${role.toLowerCase() === 'admin'
                        ? 'bg-purple-500/10 text-purple-400'
                        : 'bg-[var(--color-bg-secondary)] text-[var(--color-text-secondary)]'
                      }`}
              title={`${this.formatRole(role)} of this group`}
            >
              {role.toLowerCase() === 'admin' ? '👑 Admin' : '👤 Member'}
            </span>
          ))}
        </div>

        {group.members.length > 1 && (
          showConfirmRemove ? (
            <div class="flex items-center gap-1 shrink-0">
              <button
                onClick={() => onRemove(member.pubkey)}
                class="px-2 py-1 text-xs text-red-400 hover:text-red-300 transition-colors"
              >
                Confirm
              </button>
              <button
                onClick={onHideConfirmRemove}
                class="px-2 py-1 text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
              >
                Cancel
              </button>
            </div>
          ) : (
            <button
              onClick={onShowConfirmRemove}
              disabled={isRemoving}
              class="opacity-0 group-hover:opacity-100 shrink-0 px-2 py-1 text-xs text-red-400
                     hover:text-red-300 transition-all duration-150 disabled:opacity-50 disabled:cursor-not-allowed
                     flex items-center gap-1"
              title="Remove member"
            >
              {isRemoving ? (
                <>
                  <span class="animate-spin">⚡</span>
                  Remove
                </>
              ) : (
                <>
                  <svg class="w-3.5 h-3.5 text-red-400" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    <path d="M10 11v6M14 11v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                </>
              )}
            </button>
          )
        )}
      </div>
    )
  }
}