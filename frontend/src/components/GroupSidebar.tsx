import { Group } from "../types";
import { Component } from "preact";
import { NostrClient } from '../api/nostr_client';

interface GroupSidebarProps {
  groups: Group[];
  selectedGroupId?: string;
  onSelectGroup: (group: Group) => void;
  client: NostrClient;
}

interface GroupSidebarState {
  adminGroups: Set<string>;
  memberGroups: Set<string>;
}

export class GroupSidebar extends Component<GroupSidebarProps, GroupSidebarState> {
  state = {
    adminGroups: new Set<string>(),
    memberGroups: new Set<string>()
  };

  async componentDidMount() {
    const user = await this.props.client.ndkInstance.signer?.user();
    if (user?.pubkey) {
      const adminGroups = new Set(
        this.props.groups
          .filter(group => group.members.some(m => m.pubkey === user.pubkey && m.roles.includes('Admin')))
          .map(group => group.id)
      );
      const memberGroups = new Set(
        this.props.groups
          .filter(group => group.members.some(m => m.pubkey === user.pubkey))
          .map(group => group.id)
      );
      this.setState({ adminGroups, memberGroups });
    }
  }

  async componentDidUpdate(prevProps: GroupSidebarProps) {
    if (prevProps.groups !== this.props.groups) {
      const user = await this.props.client.ndkInstance.signer?.user();
      if (user?.pubkey) {
        const adminGroups = new Set(
          this.props.groups
            .filter(group => group.members.some(m => m.pubkey === user.pubkey && m.roles.includes('Admin')))
            .map(group => group.id)
        );
        const memberGroups = new Set(
          this.props.groups
            .filter(group => group.members.some(m => m.pubkey === user.pubkey))
            .map(group => group.id)
        );
        this.setState({ adminGroups, memberGroups });
      }
    }
  }

  render() {
    const { groups, selectedGroupId, onSelectGroup } = this.props;
    const { adminGroups, memberGroups } = this.state;

    return (
      <div class="mt-8 space-y-2">
        {groups.map(group => (
          <button
            key={group.id}
            onClick={() => onSelectGroup(group)}
            class={`w-full p-3 rounded-lg border text-left transition-colors
                   ${selectedGroupId === group.id
                     ? 'bg-[var(--color-bg-primary)] border-[var(--color-border-hover)]'
                     : 'bg-[var(--color-bg-secondary)] border-[var(--color-border)] hover:border-[var(--color-border-hover)]'
                   }`}
          >
            <div class="flex items-center gap-3">
              <div class="shrink-0 w-10 h-10 bg-[var(--color-bg-primary)] rounded-lg flex items-center justify-center text-lg overflow-hidden">
                {group.picture ? (
                  <img
                    src={group.picture}
                    alt={group.name}
                    class="w-full h-full object-cover"
                    onError={(e) => {
                      (e.target as HTMLImageElement).style.display = 'none';
                      e.currentTarget.parentElement!.textContent = group.name.charAt(0).toUpperCase();
                    }}
                  />
                ) : (
                  group.name.charAt(0).toUpperCase()
                )}
              </div>
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <h3 class="font-medium text-[var(--color-text-primary)] truncate">
                    {group.name}
                  </h3>
                  {adminGroups.has(group.id) ? (
                    <span class="shrink-0 px-1.5 py-0.5 text-[10px] font-medium bg-purple-500/10 text-purple-400 rounded-full border border-purple-500/20 flex items-center gap-1">
                      <svg class="w-2.5 h-2.5" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
                        <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" />
                      </svg>
                      Admin
                    </span>
                  ) : memberGroups.has(group.id) && (
                    <span class="shrink-0 px-1.5 py-0.5 text-[10px] font-medium bg-blue-500/10 text-blue-400 rounded-full border border-blue-500/20 flex items-center gap-1">
                      <svg class="w-2.5 h-2.5" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
                        <path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                        <path d="M9 7a4 4 0 1 0 0 8 4 4 0 0 0 0-8z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                      </svg>
                      Member
                    </span>
                  )}
                </div>
                <div class="flex items-center gap-2 text-sm text-[var(--color-text-tertiary)]">
                  <span>{group.members.length} members</span>
                  {group.private && (
                    <>
                      <span>·</span>
                      <span>Private</span>
                    </>
                  )}
                </div>
              </div>
            </div>
          </button>
        ))}

        {groups.length === 0 && (
          <div class="text-center py-12">
            <div class="mb-3 text-2xl">✨</div>
            <p class="text-sm text-[var(--color-text-tertiary)]">No groups yet</p>
            <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
              Create your first group to get started
            </p>
          </div>
        )}
      </div>
    )
  }
}