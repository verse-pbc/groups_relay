import { Component } from 'preact'
import type { Group } from '../types'

interface GroupListItemProps {
  group: Group
  onSelect?: (group: Group) => void
}

export class GroupListItem extends Component<GroupListItemProps> {
  render() {
    const { group, onSelect } = this.props

    return (
      <div
        class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] p-4 cursor-pointer hover:border-[var(--color-accent)] transition-colors"
        onClick={() => onSelect?.(group)}
      >
        <h3 class="text-lg font-semibold text-[var(--color-text-primary)]">
          {group.name || 'Unnamed Group'}
        </h3>
        {group.about && (
          <p class="text-sm text-[var(--color-text-secondary)] mt-1">
            {group.about}
          </p>
        )}
        <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
          {group.members.length} members
        </p>
      </div>
    )
  }
}