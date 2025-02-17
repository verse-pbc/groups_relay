import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { GroupInfo } from './GroupInfo'
import { GroupTimestamps } from './GroupTimestamps'
import { BaseComponent } from './BaseComponent'

interface GroupHeaderProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
  onDelete?: (groupId: string) => void
}

interface GroupHeaderState {
  isAdmin: boolean
  isEditing: boolean
}

export class GroupHeader extends BaseComponent<GroupHeaderProps, GroupHeaderState> {
  state = {
    isAdmin: false,
    isEditing: false
  }

  async componentDidMount() {
    const user = await this.props.client.ndkInstance.signer?.user();
    if (user?.pubkey) {
      const isAdmin = this.props.group.members.some(m =>
        m.pubkey === user.pubkey && m.roles.includes('Admin')
      );
      this.setState({ isAdmin });
    }
  }

  toggleEditing = () => {
    this.setState(state => ({ isEditing: !state.isEditing }))
  }

  handleEditSubmit = async (about: string) => {
    const { group, client, showMessage } = this.props

    try {
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
    const { group, client, showMessage } = this.props
    const { isEditing } = this.state

    return (
      <div class="flex-shrink-0">
        <div class="p-4">
          <GroupInfo
            group={group}
            client={client}
            showMessage={showMessage}
            isEditing={isEditing}
            onEditSubmit={this.handleEditSubmit}
            onEditCancel={this.handleEditCancel}
            onDelete={this.props.onDelete}
          />
        </div>
        <GroupTimestamps group={group} />
      </div>
    )
  }
}