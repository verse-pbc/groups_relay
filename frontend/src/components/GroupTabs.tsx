import { Component } from 'preact'
import type { Group } from '../types'

interface GroupTabsProps {
  group: Group
  activeTab: 'content' | 'members' | 'invites' | 'requests' | 'info'
  onTabChange: (tab: 'content' | 'members' | 'invites' | 'requests' | 'info') => void
}

export class GroupTabs extends Component<GroupTabsProps> {
  render() {
    const { group, activeTab, onTabChange } = this.props

    return (
      <div class="flex items-center gap-2 p-4 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)] overflow-x-auto">
        <button
          onClick={() => onTabChange('content')}
          class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
            activeTab === 'content'
              ? 'text-white bg-[var(--color-accent)]/10'
              : 'text-[#8484ac] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
          }`}
        >
          💬 Activity {group.content?.length ? `(${group.content.length})` : ''}
        </button>
        <button
          onClick={() => onTabChange('members')}
          class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
            activeTab === 'members'
              ? 'text-white bg-[var(--color-accent)]/10'
              : 'text-[#8484ac] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
          }`}
        >
          👥 Members {group.members?.length ? `(${group.members.length})` : ''}
        </button>
        <button
          onClick={() => onTabChange('invites')}
          class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
            activeTab === 'invites'
              ? 'text-white bg-[var(--color-accent)]/10'
              : 'text-[#8484ac] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
          }`}
        >
          ✉️ Invites {group.invites ? `(${Object.keys(group.invites).length})` : ''}
        </button>
        <button
          onClick={() => onTabChange('requests')}
          class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
            activeTab === 'requests'
              ? 'text-white bg-[var(--color-accent)]/10'
              : 'text-[#8484ac] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
          }`}
        >
          🔔 Requests {group.joinRequests?.length ? `(${group.joinRequests.length})` : ''}
        </button>
        <button
          onClick={() => onTabChange('info')}
          class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
            activeTab === 'info'
              ? 'text-white bg-[var(--color-accent)]/10'
              : 'text-[#8484ac] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
          }`}
        >
          ℹ️ Info
        </button>
      </div>
    )
  }
} 