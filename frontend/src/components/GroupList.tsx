import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { GroupListItem } from './GroupListItem'

interface GroupListProps {
  groups: Group[]
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onSelect?: (group: Group) => void
}

export class GroupList extends Component<GroupListProps> {
  render() {
    const { groups, onSelect } = this.props

    return (
      <div class="space-y-4">
        {groups.map(group => (
          <GroupListItem
            key={group.id}
            group={group}
            onSelect={onSelect}
          />
        ))}
      </div>
    )
  }
}