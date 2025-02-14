import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { GroupInfo } from './GroupInfo'
import { GroupTimestamps } from './GroupTimestamps'

interface GroupHeaderProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onDelete?: (groupId: string) => void
}

interface GroupHeaderState {
  showEditName: boolean
  editingName: string
  showConfirmDelete: boolean
  isDeleting: boolean
  copiedId: boolean
}

export class GroupHeader extends Component<GroupHeaderProps, GroupHeaderState> {
  private copyTimeout: number | null = null;

  state = {
    showEditName: false,
    editingName: '',
    showConfirmDelete: false,
    isDeleting: false,
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
    const { group } = this.props
    const { showEditName, editingName, showConfirmDelete, isDeleting, copiedId } = this.state

    return (
      <div class="flex-shrink-0">
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
              <div class="flex items-center gap-1">
                <button
                  onClick={this.handleDeleteGroup}
                  class="text-xs text-red-400 hover:text-red-300 transition-colors"
                >
                  Confirm
                </button>
                <span class="text-[var(--color-text-tertiary)]">·</span>
                <button
                  onClick={() => this.setState({ showConfirmDelete: false })}
                  class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                >
                  Cancel
                </button>
              </div>
            ) : (
              <button
                onClick={() => this.setState({ showConfirmDelete: true })}
                disabled={isDeleting}
                class="text-xs text-[var(--color-text-tertiary)] hover:text-red-400 transition-colors
                       flex items-center gap-1.5"
                title="Delete group"
              >
                {isDeleting ? (
                  <>
                    <span class="animate-spin">⚡</span>
                    <span>Deleting...</span>
                  </>
                ) : (
                  <>
                    <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                      <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                      <path d="M10 11v6M14 11v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  </>
                )}
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
            client={this.props.client}
            showMessage={this.props.showMessage}
          />

          <GroupTimestamps group={group} />
        </div>
      </div>
    )
  }
}