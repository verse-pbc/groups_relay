import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { GroupInfo } from './GroupInfo'
import { GroupSettings } from './GroupSettings'

interface GroupHeaderProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
}

export class GroupHeader extends Component<GroupHeaderProps> {
  render() {
    const { group, client, showMessage, updateGroupsMap } = this.props

    return (
      <div class="flex flex-col lg:flex-row lg:divide-x divide-[var(--color-border)]">
        <div class="flex-1 p-6">
          <GroupInfo
            group={group}
            client={client}
            showMessage={showMessage}
          />
        </div>
        <div class="w-full lg:w-1/3 p-6">
          <GroupSettings
            group={group}
            client={client}
            showMessage={showMessage}
            updateGroupsMap={updateGroupsMap}
          />
        </div>
      </div>
    )
  }
}