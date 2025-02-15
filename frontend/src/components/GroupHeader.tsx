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

interface GroupHeaderState {
  isEditing: boolean
}

export class GroupHeader extends Component<GroupHeaderProps, GroupHeaderState> {
  state = {
    isEditing: false
  }

  toggleEditing = () => {
    this.setState(state => ({ isEditing: !state.isEditing }))
  }

  handleEditSubmit = async (name: string, about: string) => {
    const { group, client, showMessage } = this.props

    try {
      if (name !== group.name) {
        await client.updateGroupName(group.id, name)
        group.name = name
      }

      if (about !== group.about) {
        const updatedGroup = { ...group, about }
        await client.updateGroupMetadata(updatedGroup)
        group.about = about
      }

      this.setState({ isEditing: false })
      showMessage('Group updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update group:', error)
      showMessage('Failed to update group: ' + error, 'error')
    }
  }

  handleEditCancel = () => {
    this.setState({ isEditing: false })
  }

  render() {
    const { group, client, showMessage, updateGroupsMap } = this.props
    const { isEditing } = this.state

    return (
      <div class="relative">
        {/* Settings Toggle Button */}
        <button
          onClick={this.toggleEditing}
          class="absolute top-6 right-6 text-sm text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors flex items-center gap-2"
        >
          <span class="text-lg">⚙️</span>
          Settings
        </button>

        {/* Main Content */}
        <div class="flex flex-col divide-y divide-[var(--color-border)]">
          <div class="p-6">
            <GroupInfo
              group={group}
              client={client}
              showMessage={showMessage}
              isEditing={isEditing}
              onEditSubmit={this.handleEditSubmit}
              onEditCancel={this.handleEditCancel}
            />
          </div>
          <div class={`p-6 ${isEditing ? 'block' : 'hidden'}`}>
            <GroupSettings
              group={group}
              client={client}
              showMessage={showMessage}
              updateGroupsMap={updateGroupsMap}
            />
          </div>
        </div>
      </div>
    )
  }
}