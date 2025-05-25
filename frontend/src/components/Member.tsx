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
  isTogglingRole: boolean
  isCurrentUserAdmin: boolean
}

export class Member extends Component<MemberProps, MemberState> {
  state = {
    npub: this.props.client.pubkeyToNpub(this.props.member.pubkey),
    isTogglingRole: false,
    isCurrentUserAdmin: false
  }

  async componentDidMount() {
    const user = await this.props.client.ndkInstance.signer?.user();
    if (user?.pubkey) {
      // Check if user is either a group admin or a relay admin
      const isGroupAdmin = this.props.group.members.some(m =>
        m.pubkey === user.pubkey && m.roles.some(role => role.toLowerCase() === 'admin')
      );
      const isRelayAdmin = await this.props.client.checkIsRelayAdmin();
      const isAdmin = isGroupAdmin || isRelayAdmin;
      this.setState({ isCurrentUserAdmin: isAdmin });
    }
  }

  async componentDidUpdate(prevProps: MemberProps) {
    // Check if member roles have changed or group members have changed
    if (prevProps.member.roles !== this.props.member.roles || 
        prevProps.group.members !== this.props.group.members) {
      const user = await this.props.client.ndkInstance.signer?.user();
      if (user?.pubkey) {
        const isGroupAdmin = this.props.group.members.some(m =>
          m.pubkey === user.pubkey && m.roles.some(role => role.toLowerCase() === 'admin')
        );
        const isRelayAdmin = await this.props.client.checkIsRelayAdmin();
        const isAdmin = isGroupAdmin || isRelayAdmin;
        this.setState({ isCurrentUserAdmin: isAdmin });
      }
    }
  }

  formatRole(role: string): string {
    // Preserve original case for display
    return role;
  }

  handleToggleAdmin = async () => {
    const { member, group, client, showMessage } = this.props;
    const isCurrentlyAdmin = member.roles.some(role => role.toLowerCase() === 'admin');
    const hasMultipleAdmins = group.members.filter(m =>
      m.roles.some(r => r.toLowerCase() === 'admin')
    ).length > 1;

    // Don't allow removing the last admin
    if (isCurrentlyAdmin && !hasMultipleAdmins) {
      showMessage('Cannot remove the last admin', 'error');
      return;
    }

    this.setState({ isTogglingRole: true });
    try {
      await client.toggleAdminRole(group.id, member.pubkey, !isCurrentlyAdmin);
      showMessage(`${isCurrentlyAdmin ? 'Removed admin role from' : 'Made'} user admin`, 'success');
    } catch (error) {
      console.error('Failed to toggle admin role:', error);
      showMessage(`Failed to ${isCurrentlyAdmin ? 'remove' : 'add'} admin role: ${error}`, 'error');
    } finally {
      this.setState({ isTogglingRole: false });
    }
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
    } = this.props;
    const { npub, isTogglingRole, isCurrentUserAdmin } = this.state;

    const isAdmin = member.roles.some(role => role.toLowerCase() === 'admin');

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
                        ? 'bg-purple-500/10 text-purple-400 flex items-center gap-1'
                        : 'bg-[var(--color-bg-secondary)] text-[var(--color-text-secondary)]'
                      }`}
              title={`${this.formatRole(role)} of this group`}
            >
              {role.toLowerCase() === 'admin' ? (
                <>
                  <svg class="w-3 h-3" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
                    <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" />
                  </svg>
                  Admin
                </>
              ) : (
                <>👤 Member</>
              )}
            </span>
          ))}
        </div>

        {/* Admin Controls */}
        {isCurrentUserAdmin && (
          <div class="flex items-center gap-2">
            {/* Toggle Admin Button */}
            <button
              onClick={this.handleToggleAdmin}
              disabled={isTogglingRole || (isAdmin && group.members.filter(m => m.roles.some(r => r.toLowerCase() === 'admin')).length <= 1)}
              class="shrink-0 px-2 py-1 text-xs
                     text-[var(--color-text-secondary)] hover:text-purple-400
                     transition-all duration-150 disabled:opacity-50 disabled:cursor-not-allowed
                     flex items-center gap-1 bg-[var(--color-bg-secondary)] rounded"
              title={isAdmin ? "Remove admin role" : "Make admin"}
            >
              {isTogglingRole ? (
                <span class="animate-spin">⚡</span>
              ) : (
                <svg class={`w-4 h-4 ${isAdmin ? 'text-purple-400' : ''}`} viewBox="0 0 24 24" fill={isAdmin ? "currentColor" : "none"} stroke="currentColor" xmlns="http://www.w3.org/2000/svg">
                  <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" strokeWidth={isAdmin ? "0" : "2"} />
                </svg>
              )}
            </button>

            {/* Remove Member Button - Only show if not the last admin */}
            {(!isAdmin || group.members.filter(m => m.roles.some(r => r.toLowerCase() === 'admin')).length > 1) && (
              showConfirmRemove ? (
                <div class="flex items-center gap-1 shrink-0">
                  <button
                    onClick={() => onRemove(member.pubkey)}
                    class="px-2 py-1 text-xs text-red-400 hover:text-red-300 transition-colors bg-[var(--color-bg-secondary)] rounded"
                  >
                    Confirm
                  </button>
                  <button
                    onClick={onHideConfirmRemove}
                    class="px-2 py-1 text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors bg-[var(--color-bg-secondary)] rounded"
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <button
                  onClick={onShowConfirmRemove}
                  disabled={isRemoving}
                  class="shrink-0 px-2 py-1 text-xs text-red-400
                         hover:text-red-300 transition-all duration-150 disabled:opacity-50 disabled:cursor-not-allowed
                         flex items-center gap-1 bg-[var(--color-bg-secondary)] rounded"
                  title="Remove member"
                >
                  {isRemoving ? (
                    <span class="animate-spin">⚡</span>
                  ) : (
                    <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                      <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  )}
                </button>
              )
            )}
          </div>
        )}
      </div>
    )
  }
}