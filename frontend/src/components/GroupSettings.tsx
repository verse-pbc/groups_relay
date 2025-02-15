import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'

interface GroupSettingsProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
}

interface GroupSettingsState {
  showConfirmDelete: boolean
  isDeleting: boolean
}

export class GroupSettings extends Component<GroupSettingsProps, GroupSettingsState> {
  state = {
    showConfirmDelete: false,
    isDeleting: false
  }

  handleMetadataChange = async (changes: Partial<Group>) => {
    try {
      const updatedGroup = { ...this.props.group, ...changes }
      await this.props.client.updateGroupMetadata(updatedGroup)
      this.props.updateGroupsMap(map => {
        map.set(updatedGroup.id, updatedGroup)
      })
      this.props.showMessage('Group settings updated successfully!', 'success')
    } catch (error) {
      console.error('Failed to update group settings:', error)
      this.props.showMessage('Failed to update group settings: ' + error, 'error')
    }
  }

  handleDeleteGroup = async () => {
    this.setState({ isDeleting: true })
    try {
      await this.props.client.deleteGroup(this.props.group.id)
      this.props.showMessage('Group deleted successfully', 'success')
      this.props.updateGroupsMap(map => {
        map.delete(this.props.group.id)
      })
    } catch (error) {
      console.error('Failed to delete group:', error)
      this.props.showMessage('Failed to delete group: ' + error, 'error')
    } finally {
      this.setState({ isDeleting: false, showConfirmDelete: false })
    }
  }

  render() {
    const { group } = this.props
    const { showConfirmDelete, isDeleting } = this.state

    return (
      <div class="max-w-lg space-y-6">
        {/* Settings */}
        <div>
          <div>
            <h3 class="text-base font-semibold leading-6 text-[var(--color-text-primary)] flex items-center gap-2">
              <span class="text-[var(--color-text-secondary)]">⚙️</span>
              Settings
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
                <span class="text-lg">👥</span>
                <div>
                  <div class="font-medium">Closed Group</div>
                  <div class="text-sm text-[var(--color-text-tertiary)]">Only admins can invite new members</div>
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

        {/* Delete Group */}
        <div class="flex justify-end">
          {showConfirmDelete ? (
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
          )}
        </div>
      </div>
    )
  }
} 