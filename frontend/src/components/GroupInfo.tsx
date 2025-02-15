import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { BaseComponent } from './BaseComponent'

interface GroupInfoProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  isEditing: boolean
  onEditSubmit: (name: string, about: string) => Promise<void>
  onEditCancel: () => void
}

interface GroupInfoState {
  editingName: string
  editingAbout: string
}

export class GroupInfo extends BaseComponent<GroupInfoProps, GroupInfoState> {
  state = {
    editingName: '',
    editingAbout: ''
  }

  componentWillReceiveProps(nextProps: GroupInfoProps) {
    if (nextProps.isEditing && !this.props.isEditing) {
      // Initialize form when entering edit mode
      this.setState({
        editingName: nextProps.group.name,
        editingAbout: nextProps.group.about || ''
      })
    }
  }

  handleEditSubmit = async (e: Event) => {
    e.preventDefault();
    const { editingName, editingAbout } = this.state

    if (!editingName.trim()) {
      return
    }

    await this.props.onEditSubmit(editingName, editingAbout)
  }

  handleMetadataChange = async (changes: Partial<Group>) => {
    try {
      const updatedGroup = {
        ...this.props.group,
        ...changes
      }
      await this.props.client.updateGroupMetadata(updatedGroup)

      if ('private' in changes && changes.private !== undefined) {
        this.props.group.private = changes.private
        this.props.showMessage('Group privacy updated successfully!', 'success')
      } else if ('closed' in changes && changes.closed !== undefined) {
        this.props.group.closed = changes.closed
        this.props.showMessage('Group membership setting updated successfully!', 'success')
      }
    } catch (error) {
      console.error('Failed to update group settings:', error)
      this.showError('Failed to update group settings', error)
    }
  }

  render() {
    const { group, isEditing, onEditCancel } = this.props
    const { editingName, editingAbout } = this.state

    return (
      <div class="space-y-6">
        {/* Group Avatar and Name/About */}
        <div class="flex flex-col gap-4">
          <div class="flex items-center gap-4">
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
            {!isEditing && (
              <div class="flex-1 space-y-1">
                <h2 class="text-3xl font-medium text-[var(--color-text-primary)]">{group.name}</h2>
                <p class="text-sm text-[var(--color-text-secondary)]">
                  {group.about || 'No description'}
                </p>
              </div>
            )}
          </div>

          {isEditing && (
            <form onSubmit={this.handleEditSubmit} class="w-full space-y-3">
              <div class="w-full">
                <input
                  type="text"
                  value={editingName}
                  onInput={(e: Event) => this.setState({ editingName: (e.target as HTMLInputElement).value })}
                  class="w-full px-3 py-2 text-2xl bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded"
                  placeholder="Enter group name"
                />
              </div>
              <div class="w-full">
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
                  onClick={onEditCancel}
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
          )}
        </div>

        {/* Settings */}
        <div>
          <div class="border-b border-[var(--color-border)] pb-3">
            <h3 class="text-base font-semibold leading-6 text-[var(--color-text-primary)] flex items-center gap-2">
              <span class="text-[var(--color-text-secondary)]">⚙️</span>
              Privacy & Access
            </h3>
          </div>

          <div class="mt-4 space-y-4 bg-[var(--color-bg-primary)] rounded-lg p-4">
            {/* Private Group Toggle */}
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-3">
                <span class="text-lg">🔒</span>
                <div>
                  <div class="font-medium">Private Group</div>
                  <div class="text-sm text-[var(--color-text-tertiary)]">Only members can see group content</div>
                </div>
              </div>
              <button
                type="button"
                onClick={() => this.handleMetadataChange({ private: !group.private })}
                class={`${
                  group.private ? 'bg-[var(--color-accent)]' : 'bg-[#2A2B2E]'
                } relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-[var(--color-accent)] focus:ring-offset-2`}
                role="switch"
                aria-checked={group.private}
              >
                <span class="sr-only">Private group setting</span>
                <span
                  aria-hidden="true"
                  class={`${
                    group.private ? 'translate-x-5' : 'translate-x-0'
                  } pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out`}
                />
              </button>
            </div>

            {/* Closed Group Toggle */}
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-3">
                <span class="text-lg">🔐</span>
                <div>
                  <div class="font-medium">Closed Group</div>
                  <div class="text-sm text-[var(--color-text-tertiary)]">Only admins can add new members</div>
                </div>
              </div>
              <button
                type="button"
                onClick={() => this.handleMetadataChange({ closed: !group.closed })}
                class={`${
                  group.closed ? 'bg-[var(--color-accent)]' : 'bg-[#2A2B2E]'
                } relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-[var(--color-accent)] focus:ring-offset-2`}
                role="switch"
                aria-checked={group.closed}
              >
                <span class="sr-only">Closed group setting</span>
                <span
                  aria-hidden="true"
                  class={`${
                    group.closed ? 'translate-x-5' : 'translate-x-0'
                  } pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out`}
                />
              </button>
            </div>
          </div>
        </div>
      </div>
    )
  }
}