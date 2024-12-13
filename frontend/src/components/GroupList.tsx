import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { GroupCard } from './GroupCard'

interface GroupListProps {
  groups: Group[]
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

export class GroupList extends Component<GroupListProps> {
  render() {
    const { groups, client, showMessage } = this.props

    return (
      <div class="space-y-4">
        {groups.map(group => (
          <GroupCard key={group.id} group={group} client={client} showMessage={showMessage} />
        ))}
      </div>
    )
  }
}