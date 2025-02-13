import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { GroupTimestamps } from './GroupTimestamps'

interface GroupInfoProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

interface GroupInfoState {
  showEditName: boolean
  editingName: string
  isEditingAbout: boolean
  newAbout: string
  copiedId: boolean
}

export class GroupInfo extends Component<GroupInfoProps, GroupInfoState> {
  private copyTimeout: number | null = null;

  state = {
    showEditName: false,
    editingName: '',
    isEditingAbout: false,
    newAbout: '',
    copiedId: false
  }

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
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

  render() {
    const { group } = this.props
    const { showEditName, editingName, isEditingAbout, newAbout, copiedId } = this.state

    return (
      <div class="space-y-4">
        {/* Group Avatar and Name */}
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

        {/* Group ID */}
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

        {/* About Section */}
        <div class="space-y-3">
          <div class="flex items-center justify-between">
            <h3 class="text-sm font-medium text-[var(--color-text-secondary)]">About</h3>
            {!isEditingAbout && (
              <button
                onClick={() => this.setState({ isEditingAbout: true, newAbout: group.about || '' })}
                class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
              >
                Edit
              </button>
            )}
          </div>
          {isEditingAbout ? (
            <div class="space-y-2">
              <textarea
                value={newAbout}
                onInput={(e) => this.setState({ newAbout: (e.target as HTMLTextAreaElement).value })}
                rows={3}
                class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)]
                       text-sm rounded-lg text-[var(--color-text-primary)]
                       placeholder-[var(--color-text-tertiary)]
                       focus:outline-none focus:ring-1 focus:ring-accent
                       hover:border-[var(--color-border-hover)] transition-colors resize-none"
                placeholder="Enter group description"
              />
              <div class="flex justify-end gap-2">
                <button
                  onClick={() => this.setState({ isEditingAbout: false })}
                  class="px-2 py-1 text-xs text-[var(--color-text-tertiary)]
                         hover:text-[var(--color-text-secondary)] transition-colors"
                >
                  Cancel
                </button>
                <button
                  onClick={this.handleAboutSave}
                  class="px-2 py-1 text-xs text-accent hover:text-accent/90 transition-colors"
                >
                  Save
                </button>
              </div>
            </div>
          ) : (
            <p class="text-sm text-[var(--color-text-secondary)]">
              {group.about || 'No description'}
            </p>
          )}
        </div>

        <GroupTimestamps group={group} />
      </div>
    )
  }
}