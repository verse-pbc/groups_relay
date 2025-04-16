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

interface GroupCardState {
  isRelayAdmin: boolean
}

export class GroupCard extends Component<GroupCardProps, GroupCardState> {
  state = {
    isRelayAdmin: false
  }

  async componentDidMount() {
    try {
      const isAdmin = await this.props.client.checkIsRelayAdmin();
      if (isAdmin) {
        this.setState({ isRelayAdmin: true });
      }
    } catch (error) {
      console.error('Failed to check relay admin status:', error);
    }
  }

  render() {
    const { group, client, showMessage, onDelete, updateGroupsMap } = this.props
    const { isRelayAdmin } = this.state

    return (
      <article class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] flex flex-col">
        {isRelayAdmin && (
          <div class="p-4 bg-yellow-500/10 border-b border-yellow-500/20">
            <div class="flex items-center gap-2 text-yellow-500">
              <svg class="w-5 h-5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
              </svg>
              <div class="text-sm">
                <span class="font-medium">Warning:</span> You are using the relay admin key. You have full administrative power over all groups.
              </div>
            </div>
          </div>
        )}
        <GroupHeader
          group={group}
          client={client}
          showMessage={showMessage}
          onDelete={onDelete}
          updateGroupsMap={updateGroupsMap}
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