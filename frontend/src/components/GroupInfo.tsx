import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'

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

export class GroupInfo extends Component<GroupInfoProps, GroupInfoState> {
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

  render() {
    const { group, isEditing, onEditCancel } = this.props
    const { editingName, editingAbout } = this.state

    return (
      <div class="space-y-4">
        {/* Group Avatar and Name/About */}
        <div class={`flex ${isEditing ? 'items-start' : 'items-center'} gap-4`}>
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
            ) : (
              <div class="space-y-1">
                <h2 class="text-3xl font-medium text-[var(--color-text-primary)]">{group.name}</h2>
                <p class="text-sm text-[var(--color-text-secondary)]">
                  {group.about || 'No description'}
                </p>
              </div>
            )}
          </div>
        </div>
      </div>
    )
  }
}