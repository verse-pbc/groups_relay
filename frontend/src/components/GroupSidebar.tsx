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
    const { selectedGroupId, onSelectGroup } = this.props;
    const { adminGroups, memberGroups } = this.state;

    return (
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
                <span class="shrink-0 px-1.5 py-0.5 text-[10px] font-medium bg-purple-500/10 text-purple-400 rounded-full border border-purple-500/20">
                  Admin
                </span>
              ) : memberGroups.has(group.id) && (
                <span class="shrink-0 px-1.5 py-0.5 text-[10px] font-medium bg-blue-500/10 text-blue-400 rounded-full border border-blue-500/20">
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
    );
  };

  render() {
    const { groups } = this.props;
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
      <div class="mt-8 space-y-6">
        {/* Main Groups */}
        <div class="space-y-2">
          {mainGroups.map(this.renderGroupButton)}
        </div>

        {/* Other Groups */}
        {otherGroups.length > 0 && (
          <div class="space-y-2">
            <button
              onClick={this.toggleOtherGroups}
              class="w-full flex items-center justify-between text-sm font-medium text-[var(--color-text-secondary)] px-1 hover:text-[var(--color-text-primary)] transition-colors"
            >
              <div class="flex items-center gap-2">
                <span>Other groups</span>
                <span class="text-xs text-[var(--color-text-tertiary)]">({otherGroups.length})</span>
              </div>
              <svg
                class={`w-4 h-4 transition-transform duration-200 ${showOtherGroups ? 'rotate-180' : ''}`}
                viewBox="0 0 24 24"
                fill="none"
                xmlns="http://www.w3.org/2000/svg"
              >
                <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
            </button>
            {showOtherGroups && (
              <div class="space-y-2">
                {otherGroups.map(this.renderGroupButton)}
              </div>
            )}
          </div>
        )}
      </div>
    );
  }
}