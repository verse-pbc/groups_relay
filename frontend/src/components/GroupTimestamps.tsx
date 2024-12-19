import { Component } from 'preact'
import type { Group } from '../types'

interface GroupTimestampsProps {
  group: Group
}

export class GroupTimestamps extends Component<GroupTimestampsProps> {
  render() {
    const { group } = this.props

    return (
      <div class="space-y-1">
        <div>
          <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">Created:</span>
          <div class="text-xs text-[var(--color-text-secondary)]">
            {new Date(group.created_at * 1000).toLocaleString()}
          </div>
        </div>
        <div>
          <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">Updated:</span>
          <div class="text-xs text-[var(--color-text-secondary)]">
            {new Date(group.updated_at * 1000).toLocaleString()}
          </div>
        </div>
      </div>
    )
  }
}