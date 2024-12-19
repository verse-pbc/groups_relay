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
  activeTab: 'members' | 'invites' | 'requests'
  copiedId: boolean
}

export class GroupCard extends Component<GroupCardProps, GroupCardState> {
  private copyTimeout: number | null = null;

  constructor(props: GroupCardProps) {
    super(props)
    this.state = {
      isEditingName: false,
      newName: props.group.name || '',
      isEditingAbout: false,
      newAbout: props.group.about || '',
      newMemberPubkey: '',
      isAddingMember: false,
      activeTab: 'members',
      copiedId: false
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
      this.props.group.name = this.state.newName
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
      this.props.group.about = this.state.newAbout
      this.setState({ isEditingAbout: false })
      this.props.showMessage('Group description updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update about:', error)
      this.props.showMessage('Failed to update group description: ' + error, 'error')
    }
  }

  handleMetadataChange = async (changes: Partial<Group>) => {
    try {
      const updatedGroup = { ...this.props.group, ...changes }
      await this.props.client.updateGroupMetadata(updatedGroup)
      Object.assign(this.props.group, changes)
      this.props.showMessage('Group settings updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update group settings:', error)
      this.props.showMessage('Failed to update group settings: ' + error, 'error')
    }
  }

  copyGroupId = () => {
    navigator.clipboard.writeText(this.props.group.id)
    this.setState({ copiedId: true })

    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }

    this.copyTimeout = window.setTimeout(() => {
      this.setState({ copiedId: false })
    }, 2000)
  }

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }
  }

  render() {
    const { group, client } = this.props
    const { isEditingName, newName, isEditingAbout, newAbout, activeTab, copiedId } = this.state

    return (
      <article class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] overflow-hidden">
        <div class="flex flex-col lg:flex-row lg:divide-x divide-[var(--color-border)]">
          {/* Left Column - Group Info */}
          <div class="lg:w-1/3 flex flex-col">
            <GroupHeader
              group={group}
              isEditingName={isEditingName}
              newName={newName}
              onNameEdit={this.handleNameEdit}
              onNameSave={this.handleNameSave}
              onNameChange={(name) => this.setState({ newName: name })}
            />

            <div class="p-4 flex-grow space-y-4">
              {/* Group ID with copy button */}
              <div class="space-y-1">
                <label class="text-xs font-medium text-[var(--color-text-secondary)]">Group ID</label>
                <button
                  onClick={this.copyGroupId}
                  class="w-full px-3 py-2 bg-[var(--color-bg-tertiary)] rounded
                         text-xs font-mono text-[var(--color-text-secondary)]
                         hover:text-[var(--color-text-primary)] transition-colors
                         flex items-center justify-between gap-2 border border-[var(--color-border)]"
                >
                  <span class="truncate">{group.id}</span>
                  <span class="flex-shrink-0 text-xs">
                    {copiedId ? '✓ Copied!' : '📋 Copy'}
                  </span>
                </button>
              </div>

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

          {/* Middle Column - Members & Actions */}
          <div class="lg:w-1/3">
            {/* Tabs */}
            <div class="border-b border-[var(--color-border)] px-2">
              <div class="flex -mb-px">
                {(['members', 'invites', 'requests'] as const).map(tab => (
                  <button
                    key={tab}
                    onClick={() => this.setState({ activeTab: tab })}
                    class={`px-4 py-2 text-sm font-medium border-b-2 transition-colors
                            ${activeTab === tab
                              ? 'border-accent text-accent'
                              : 'border-transparent text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)]'
                            }`}
                  >
                    {tab.charAt(0).toUpperCase() + tab.slice(1)}
                  </button>
                ))}
              </div>
            </div>

            {/* Tab Content */}
            <div class="p-4">
              {activeTab === 'members' && (
                <MembersSection
                  group={group}
                  client={client}
                />
              )}
              {activeTab === 'invites' && (
                <InviteSection
                  group={group}
                  client={client}
                />
              )}
              {activeTab === 'requests' && (
                <JoinRequestSection
                  group={group}
                  client={client}
                />
              )}
            </div>
          </div>

          {/* Right Column - Content */}
          <div class="lg:w-1/3">
            <ContentSection
              group={group}
              client={client}
            />
          </div>
        </div>
      </article>
    )
  }
}