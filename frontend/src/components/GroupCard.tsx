import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { GroupHeader } from './GroupHeader'
import { GroupContent } from './GroupContent'

interface GroupCardProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onDelete?: (groupId: string) => void
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
}

export class GroupCard extends Component<GroupCardProps> {
  render() {
    const { group, client, showMessage, onDelete, updateGroupsMap } = this.props

    return (
      <article class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] overflow-hidden flex flex-col">
        <GroupHeader
          group={group}
          client={client}
          showMessage={showMessage}
          onDelete={onDelete}
        />
        <GroupContent
          group={group}
          client={client}
          showMessage={showMessage}
          updateGroupsMap={updateGroupsMap}
        />
      </article>
    )
  }
}