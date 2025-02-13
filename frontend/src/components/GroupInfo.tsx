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
  isEditing: boolean
  editingName: string
  editingAbout: string
  copiedId: boolean
}

export class GroupInfo extends Component<GroupInfoProps, GroupInfoState> {
  private copyTimeout: number | null = null;

  state = {
    isEditing: false,
    editingName: '',
    editingAbout: '',
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

  handleEditSubmit = async (e: Event) => {
    e.preventDefault();
    const { editingName, editingAbout } = this.state
    const { group } = this.props

    if (!editingName.trim()) {
      return
    }

    try {
      if (editingName !== group.name) {
        await this.props.client.updateGroupName(group.id, editingName)
        group.name = editingName
      }

      if (editingAbout !== group.about) {
        const updatedGroup = { ...group, about: editingAbout }
        await this.props.client.updateGroupMetadata(updatedGroup)
        group.about = editingAbout
      }

      this.setState({ isEditing: false })
      this.props.showMessage('Group updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update group:', error)
      this.props.showMessage('Failed to update group: ' + error, 'error')
    }
  }

  startEditing = () => {
    this.setState({
      isEditing: true,
      editingName: this.props.group.name,
      editingAbout: this.props.group.about || ''
    })
  }

  render() {
    const { group } = this.props
    const { isEditing, editingName, editingAbout, copiedId } = this.state

    return (
      <div class="space-y-4">
        {/* Group Avatar and Name/About */}
        <div class="flex items-start gap-4">
          <div class="w-20 h-20 bg-[var(--color-bg-primary)] rounded-full flex items-center justify-center text-3xl overflow-hidden border border-[var(--color-border)]">
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
          <div class="flex-1">
            {isEditing ? (
              <form onSubmit={this.handleEditSubmit} class="space-y-3">
                <div>
                  <input
                    type="text"
                    value={editingName}
                    onInput={(e: Event) => this.setState({ editingName: (e.target as HTMLInputElement).value })}
                    class="w-full px-3 py-2 text-2xl bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded"
                    placeholder="Enter group name"
                  />
                </div>
                <div>
                  <textarea
                    value={editingAbout}
                    onInput={(e) => this.setState({ editingAbout: (e.target as HTMLTextAreaElement).value })}
                    rows={3}
                    class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)]
                           text-sm rounded-lg text-[var(--color-text-primary)]
                           placeholder-[var(--color-text-tertiary)]
                           focus:outline-none focus:ring-1 focus:ring-accent
                           hover:border-[var(--color-border-hover)] transition-colors resize-none"
                    placeholder="Enter group description"
                  />
                </div>
                <div class="flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => this.setState({ isEditing: false })}
                    class="px-2 py-1 text-sm text-[var(--color-text-tertiary)]
                           hover:text-[var(--color-text-secondary)] transition-colors"
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    class="px-2 py-1 text-sm text-accent hover:text-accent/90 transition-colors"
                  >
                    Save
                  </button>
                </div>
              </form>
            ) : (
              <div class="flex items-start justify-between">
                <div class="space-y-1">
                  <h2 class="text-3xl font-medium text-[var(--color-text-primary)]">{group.name}</h2>
                  <p class="text-sm text-[var(--color-text-secondary)]">
                    {group.about || 'No description'}
                  </p>
                </div>
                <button
                  onClick={this.startEditing}
                  class="text-sm text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
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

        <GroupTimestamps group={group} />
      </div>
    )
  }
}