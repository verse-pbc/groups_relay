import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { InviteSection } from './InviteSection'
import { JoinRequestSection } from './JoinRequestSection'
import { ContentSection } from './ContentSection'
import { GroupHeader } from './GroupHeader'
import { GroupInfo } from './GroupInfo'
import { GroupTimestamps } from './GroupTimestamps'
import { MembersSection } from './MembersSection'

interface GroupCardProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

interface GroupCardState {
  isEditingName: boolean
  newName: string
  isEditingAbout: boolean
  newAbout: string
  newMemberPubkey: string
  isAddingMember: boolean
}

export class GroupCard extends Component<GroupCardProps, GroupCardState> {
  constructor(props: GroupCardProps) {
    super(props)
    this.state = {
      isEditingName: false,
      newName: props.group.name || '',
      isEditingAbout: false,
      newAbout: props.group.about || '',
      newMemberPubkey: '',
      isAddingMember: false
    }
  }

  handleNameEdit = () => {
    this.setState({ isEditingName: true })
  }

  handleNameSave = async () => {
    if (!this.state.newName.trim() || this.state.newName === this.props.group.name) {
      this.setState({ isEditingName: false })
      return
    }

    try {
      await this.props.client.updateGroupName(this.props.group.id, this.state.newName)
      this.props.group.name = this.state.newName // Direct modification as discussed
      this.setState({ isEditingName: false })
      this.props.showMessage('Group name updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update group name:', error)
      this.props.showMessage('Failed to update group name: ' + error, 'error')
    }
  }

  handleAboutEdit = () => {
    this.setState({ isEditingAbout: true })
  }

  handleAboutSave = async () => {
    if (this.state.newAbout === this.props.group.about) {
      this.setState({ isEditingAbout: false })
      return
    }

    try {
      const updatedGroup = { ...this.props.group, about: this.state.newAbout }
      await this.props.client.updateGroupMetadata(updatedGroup)
      this.props.group.about = this.state.newAbout // Direct modification as discussed
      this.setState({ isEditingAbout: false })
      this.props.showMessage('Group description updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update about:', error)
      this.props.showMessage('Failed to update group description: ' + error, 'error')
    }
  }

  handleMetadataChange = async (field: 'private' | 'closed', value: boolean) => {
    try {
      const updatedGroup = { ...this.props.group, [field]: value }
      await this.props.client.updateGroupMetadata(updatedGroup)
      this.props.group[field] = value
      this.props.showMessage(`Group ${field} setting updated successfully!`, 'success')
    } catch (error) {
      console.error('Error updating metadata:', error)
      this.props.showMessage(`Failed to update group ${field} setting: ` + error, 'error')
    }
  }

  handleRemoveMember = async (pubkey: string) => {
    try {
      await this.props.client.removeMember(this.props.group.id, pubkey)
      this.props.showMessage('Member removed successfully!', 'success')
    } catch (error) {
      console.error('Failed to remove member:', error)
      this.props.showMessage('Failed to remove member: ' + error, 'error')
    }
  }

  handleAddMember = async (e: Event) => {
    e.preventDefault()
    if (!this.state.newMemberPubkey.trim()) return

    this.setState({ isAddingMember: true })
    try {
      await this.props.client.addMember(this.props.group.id, this.state.newMemberPubkey)
      this.setState({ newMemberPubkey: '' })
      this.props.showMessage('Member added successfully!', 'success')
    } catch (error) {
      console.error('Failed to add member:', error)
      this.props.showMessage('Failed to add member: ' + error, 'error')
    } finally {
      this.setState({ isAddingMember: false })
    }
  }

  render() {
    const { group, client } = this.props
    const { isEditingName, newName, isEditingAbout, newAbout, newMemberPubkey, isAddingMember } = this.state

    return (
      <article class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] overflow-hidden">
        <div class="flex flex-col lg:flex-row lg:divide-x divide-[var(--color-border)]">
          <div class="lg:w-1/3 flex flex-col">
            <GroupHeader
              group={group}
              isEditingName={isEditingName}
              newName={newName}
              onNameEdit={this.handleNameEdit}
              onNameSave={this.handleNameSave}
              onNameChange={(name) => this.setState({ newName: name })}
            />

            <div class="p-3 flex-grow">
              <div class="space-y-3">
                <GroupInfo
                  group={group}
                  isEditingAbout={isEditingAbout}
                  newAbout={newAbout}
                  onAboutEdit={this.handleAboutEdit}
                  onAboutSave={this.handleAboutSave}
                  onAboutChange={(about) => this.setState({ newAbout: about })}
                  onMetadataChange={this.handleMetadataChange}
                />
                <GroupTimestamps group={group} />
              </div>
            </div>
          </div>

          <div class="lg:w-2/3 flex flex-col">
            <div class="flex flex-row divide-x divide-[var(--color-border)]">
              <div class="w-1/2">
                <MembersSection
                  group={group}
                  newMemberPubkey={newMemberPubkey}
                  isAddingMember={isAddingMember}
                  onMemberPubkeyChange={(pubkey) => this.setState({ newMemberPubkey: pubkey })}
                  onAddMember={this.handleAddMember}
                  onRemoveMember={this.handleRemoveMember}
                />
              </div>

              <div class="w-1/2 flex flex-col">
                <InviteSection group={group} client={client} />
                <JoinRequestSection group={group} client={client} />
              </div>
            </div>

            <ContentSection group={group} />
          </div>
        </div>
      </article>
    )
  }
}