import { Group } from "../types";
import { Component } from "preact";
import { NostrClient } from '../api/nostr_client';

interface GroupSidebarProps {
  groups: Group[];
  selectedGroupId?: string;
  onSelectGroup: (group: Group | string) => void;
  client: NostrClient;
  isLoading: boolean;
}

interface GroupSidebarState {
  adminGroups: Set<string>;
  memberGroups: Set<string>;
  showOtherGroups: boolean;
}

export class GroupSidebar extends Component<GroupSidebarProps, GroupSidebarState> {
  state = {
    adminGroups: new Set<string>(),
    memberGroups: new Set<string>(),
    showOtherGroups: false
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
      // Expand other groups by default if user has no groups where they are a member
      this.setState({
        adminGroups,
        memberGroups,
        showOtherGroups: memberGroups.size === 0
      });
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
        this.setState({
          adminGroups,
          memberGroups,
          showOtherGroups: memberGroups.size === 0
        });
      }
    }
  }

  toggleOtherGroups = () => {
    this.setState(state => ({ showOtherGroups: !state.showOtherGroups }));
  };

  renderGroupButton = (group: Group) => {
    const { selectedGroupId: _selectedGroupId, onSelectGroup: _onSelectGroup } = this.props;
    const { adminGroups } = this.state;

    return (
      <button
        key={group.id}
        onClick={() => _onSelectGroup(group)}
        class={`w-full px-3 py-3 rounded-lg text-left transition-all duration-200
               ${_selectedGroupId === group.id
                 ? 'bg-white/8 text-[var(--color-text-primary)]'
                 : 'text-[var(--color-text-secondary)] hover:bg-white/4 hover:text-[var(--color-text-primary)]'
               }`}
      >
        <div class="flex items-center gap-3">
          <div class="shrink-0 w-8 h-8 bg-white/10 rounded-lg flex items-center justify-center text-sm font-medium overflow-hidden">
            {group.picture ? (
              <img
                src={group.picture}
                alt={group.name}
                class="w-full h-full object-cover rounded-lg"
                onError={(e) => {
                  (e.target as HTMLImageElement).style.display = 'none';
                  e.currentTarget.parentElement!.textContent = group.name.charAt(0).toUpperCase();
                }}
              />
            ) : (
              group.name.charAt(0).toUpperCase()
            )}
          </div>
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2 mb-0.5">
              <h3 class="font-medium truncate text-sm">
                {group.name}
              </h3>
              {adminGroups.has(group.id) && (
                <span class="shrink-0 px-1.5 py-0.5 text-[9px] font-medium bg-purple-400/20 text-purple-300 rounded">
                  Admin
                </span>
              )}
            </div>
            <div class="text-xs opacity-60">
              {group.members.length} member{group.members.length !== 1 ? 's' : ''}
              {group.private && ' • Private'}
            </div>
          </div>
        </div>
      </button>
    );
  };

  render() {
    const { groups, selectedGroupId: _selectedGroupId, onSelectGroup: _onSelectGroup, isLoading } = this.props;
    const { adminGroups, memberGroups, showOtherGroups } = this.state;

    // Empty groups where user is not a member
    const otherGroups = groups.filter(g =>
      !adminGroups.has(g.id) &&
      !memberGroups.has(g.id) &&
      (!g.content?.length || g.content.length === 0)
    );

    // All groups except empty non-member groups
    const mainGroups = groups
      .filter(g => !otherGroups.includes(g))
      .sort((a, b) => {
        const aIsAdmin = adminGroups.has(a.id);
        const bIsAdmin = adminGroups.has(b.id);
        const aIsMember = memberGroups.has(a.id);
        const bIsMember = memberGroups.has(b.id);

        if (aIsAdmin !== bIsAdmin) return aIsAdmin ? -1 : 1;
        if (aIsMember !== bIsMember) return aIsMember ? -1 : 1;
        return b.updated_at - a.updated_at;
      });

    if (groups.length === 0) {
      return (
        <div class="mt-8 text-center py-12">
          <div class="mb-3 text-2xl">✨</div>
          <p class="text-sm text-[var(--color-text-tertiary)]">No groups yet</p>
          <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
            Create your first group to get started
          </p>
        </div>
      );
    }

    return (
      <div class="mt-8">
        {/* Channels Header - more prominent */}
        <h3 class="text-base font-semibold text-[var(--color-text-primary)] uppercase tracking-wider mb-4 px-3">Channels</h3>
        
        {/* Main Groups */}
        <div class="space-y-1 px-3">
          {isLoading ? (
            <div class="text-center text-sm text-[var(--color-text-secondary)] py-6 opacity-60">
              Loading channels...
            </div>
          ) : groups.length > 0 ? (
            mainGroups.map(this.renderGroupButton)
          ) : (
            <div class="text-center text-sm text-[var(--color-text-secondary)] py-6 opacity-40">
              No channels found
            </div>
          )}
        </div>

        {/* Other Groups */}
        {otherGroups.length > 0 && (
          <div class="mt-6">
            <button
              onClick={this.toggleOtherGroups}
              class="w-full flex items-center justify-between text-sm text-[var(--color-text-secondary)]/80 px-3 py-2 hover:text-[var(--color-text-secondary)] transition-colors"
            >
              <div class="flex items-center gap-2">
                <span>Other channels</span>
                <span class="text-xs opacity-60">({otherGroups.length})</span>
              </div>
              <svg
                class={`w-3 h-3 transition-transform duration-200 ${showOtherGroups ? 'rotate-180' : ''}`}
                viewBox="0 0 24 24"
                fill="none"
                xmlns="http://www.w3.org/2000/svg"
              >
                <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </button>
            {showOtherGroups && (
              <div class="space-y-1 px-3 mt-2">
                {otherGroups.map(this.renderGroupButton)}
              </div>
            )}
          </div>
        )}
      </div>
    );
  }
}