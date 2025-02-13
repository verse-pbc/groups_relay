import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { InviteSection } from './InviteSection'
import { JoinRequestSection } from './JoinRequestSection'
import { ContentSection } from './ContentSection'
import { GroupInfo } from './GroupInfo'
import { GroupTimestamps } from './GroupTimestamps'
import { MembersSection } from './MembersSection'

interface GroupCardProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onDelete?: (groupId: string) => void
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
}

interface GroupCardState {
  isEditingName: boolean
  newName: string
  isEditingAbout: boolean
  newAbout: string
  newMemberPubkey: string
  isAddingMember: boolean
  activeTab: 'members' | 'invites' | 'requests' | 'content'
  copiedId: boolean
  showEditName: boolean
  editingName: string
  showConfirmDelete: boolean
  isDeleting: boolean
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
      activeTab: 'content',
      copiedId: false,
      showEditName: false,
      editingName: '',
      showConfirmDelete: false,
      isDeleting: false
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

  handleNameSubmit = async (e: Event) => {
    e.preventDefault();
    if (!this.state.editingName.trim() || this.state.editingName === this.props.group.name) {
      this.setState({ showEditName: false })
      return
    }

    try {
      await this.props.client.updateGroupName(this.props.group.id, this.state.editingName)
      this.props.group.name = this.state.editingName
      this.setState({ showEditName: false })
      this.props.showMessage('Group name updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update group name:', error)
      this.props.showMessage('Failed to update group name: ' + error, 'error')
    }
  }

  handleDeleteGroup = async () => {
    this.setState({ isDeleting: true })
    try {
      await this.props.client.deleteGroup(this.props.group.id)
      this.props.showMessage('Group deleted successfully', 'success')
      this.props.onDelete?.(this.props.group.id)
    } catch (error) {
      console.error('Failed to delete group:', error)
      this.props.showMessage('Failed to delete group: ' + error, 'error')
    } finally {
      this.setState({ isDeleting: false, showConfirmDelete: false })
    }
  }

  render() {
    const { group, client } = this.props
    const { activeTab, copiedId, showEditName, editingName, showConfirmDelete, isDeleting } = this.state

    return (
      <article class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] overflow-hidden">
        <div class="flex flex-col lg:flex-row lg:divide-x divide-[var(--color-border)]">
          {/* Left Column - Group Info */}
          <div class="w-full lg:w-[300px] flex-shrink-0">
            <div class="flex items-center justify-between p-4 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
              <div class="flex items-center gap-3">
                <div class="w-10 h-10 bg-[var(--color-bg-primary)] rounded-lg flex items-center justify-center text-lg overflow-hidden">
                  {group.picture ? (
                    <img 
                      src={group.picture} 
                      alt={group.name || 'Group'} 
                      class="w-full h-full object-cover"
                      onError={(e) => {
                        (e.target as HTMLImageElement).style.display = 'none';
                        e.currentTarget.parentElement!.textContent = group.name?.charAt(0).toUpperCase() || 'G';
                      }}
                    />
                  ) : (
                    group.name?.charAt(0).toUpperCase() || 'G'
                  )}
                </div>
                <div>
                  {showEditName ? (
                    <form onSubmit={this.handleNameSubmit} class="flex items-center gap-2">
                      <input
                        type="text"
                        value={editingName}
                        onInput={(e: Event) => this.setState({ editingName: (e.target as HTMLInputElement).value })}
                        class="px-2 py-1 text-sm bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded"
                        placeholder="Enter group name"
                      />
                      <div class="flex items-center gap-2">
                        <button
                          type="submit"
                          class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                        >
                          Save
                        </button>
                        <button
                          type="button"
                          onClick={() => this.setState({ showEditName: false })}
                          class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                        >
                          Cancel
                        </button>
                        {showConfirmDelete ? (
                          <div class="flex items-center gap-2 text-xs ml-4">
                            <button
                              onClick={this.handleDeleteGroup}
                              disabled={isDeleting}
                              class="text-red-400 hover:text-red-300 transition-colors"
                            >
                              {isDeleting ? <span class="animate-spin">⚡</span> : 'Delete'}
                            </button>
                            <span class="text-[var(--color-text-tertiary)]">·</span>
                            <button
                              onClick={() => this.setState({ showConfirmDelete: false })}
                              class="text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                            >
                              Cancel
                            </button>
                          </div>
                        ) : (
                          <button
                            onClick={() => this.setState({ showConfirmDelete: true })}
                            class="text-xs text-[var(--color-text-tertiary)] hover:text-red-400 transition-colors ml-4"
                          >
                            Delete Group
                          </button>
                        )}
                      </div>
                    </form>
                  ) : (
                    <div class="flex items-center gap-2">
                      <h2 class="text-lg font-medium text-[var(--color-text-primary)]">{group.name}</h2>
                      <button
                        onClick={() => this.setState({ showEditName: true, editingName: group.name })}
                        class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                      >
                        Edit
                      </button>
                    </div>
                  )}
                </div>
              </div>

              {!showEditName && (
                showConfirmDelete ? (
                  <div class="flex items-center gap-2 text-xs">
                    <button
                      onClick={this.handleDeleteGroup}
                      disabled={isDeleting}
                      class="text-red-400 hover:text-red-300 transition-colors"
                    >
                      {isDeleting ? <span class="animate-spin">⚡</span> : 'Delete'}
                    </button>
                    <span class="text-[var(--color-text-tertiary)]">·</span>
                    <button
                      onClick={() => this.setState({ showConfirmDelete: false })}
                      class="text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                    >
                      Cancel
                    </button>
                  </div>
                ) : (
                  <button
                    onClick={() => this.setState({ showConfirmDelete: true })}
                    class="text-xs text-[var(--color-text-tertiary)] hover:text-red-400 transition-colors"
                  >
                    Delete Group
                  </button>
                )
              )}
            </div>

            <div class="p-4 flex-grow space-y-4">
              {/* Group ID with copy button */}
              <div class="space-y-1">
                <label class="block text-sm font-medium text-[var(--color-text-secondary)]">
                  Group ID
                </label>
                <div class="flex items-center gap-2">
                  <code class="flex-1 px-2 py-1 text-sm bg-[var(--color-bg-primary)] rounded font-mono">
                    {group.id}
                  </code>
                  <button
                    onClick={this.copyGroupId}
                    class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                  >
                    {copiedId ? 'Copied!' : 'Copy'}
                  </button>
                </div>
              </div>

              <GroupInfo
                group={group}
                isEditingAbout={this.state.isEditingAbout}
                newAbout={this.state.newAbout}
                onAboutEdit={this.handleAboutEdit}
                onAboutSave={this.handleAboutSave}
                onAboutChange={(about) => this.setState({ newAbout: about })}
                onMetadataChange={this.handleMetadataChange}
              />

              <GroupTimestamps group={group} />
            </div>
          </div>

          {/* Right Column - Content */}
          <div class="w-full lg:w-2/3 flex flex-col w-0 flex-1">
            <div class="flex items-center gap-2 p-4 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)] overflow-x-auto">
              <button
                onClick={() => this.setState({ activeTab: 'content' })}
                class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
                  activeTab === 'content'
                    ? 'text-[var(--color-accent)] bg-[var(--color-accent)]/10'
                    : 'text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
                }`}
              >
                Content {group.content?.length ? `(${group.content.length})` : ''}
              </button>
              <button
                onClick={() => this.setState({ activeTab: 'members' })}
                class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
                  activeTab === 'members'
                    ? 'text-[var(--color-accent)] bg-[var(--color-accent)]/10'
                    : 'text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
                }`}
              >
                Members {group.members?.length ? `(${group.members.length})` : ''}
              </button>
              <button
                onClick={() => this.setState({ activeTab: 'invites' })}
                class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
                  activeTab === 'invites'
                    ? 'text-[var(--color-accent)] bg-[var(--color-accent)]/10'
                    : 'text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
                }`}
              >
                Invites {group.invites ? `(${Object.keys(group.invites).length})` : ''}
              </button>
              <button
                onClick={() => this.setState({ activeTab: 'requests' })}
                class={`shrink-0 text-sm font-medium px-3 py-1.5 rounded-full transition-all ${
                  activeTab === 'requests'
                    ? 'text-[var(--color-accent)] bg-[var(--color-accent)]/10'
                    : 'text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-primary)]'
                }`}
              >
                Requests {group.joinRequests?.length ? `(${group.joinRequests.length})` : ''}
              </button>
            </div>

            <div class="flex-grow">
              {activeTab === 'content' && (
                <ContentSection
                  group={group}
                  client={client}
                  showMessage={this.props.showMessage}
                />
              )}
              {activeTab === 'members' && (
                <MembersSection
                  group={group}
                  client={client}
                  showMessage={this.props.showMessage}
                />
              )}
              {activeTab === 'invites' && (
                <InviteSection
                  group={group}
                  client={client}
                  updateGroupsMap={this.props.updateGroupsMap}
                  showMessage={this.props.showMessage}
                />
              )}
              {activeTab === 'requests' && (
                <JoinRequestSection
                  group={group}
                  client={client}
                  showMessage={this.props.showMessage}
                />
              )}
            </div>
          </div>
        </div>
      </article>
    )
  }
}