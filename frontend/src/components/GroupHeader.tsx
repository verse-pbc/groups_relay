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
  showEditName: boolean
  editingName: string
  showConfirmDelete: boolean
  isDeleting: boolean
  showEditImage: boolean
  editingImage: string
  isUpdatingImage: boolean
  isAdmin: boolean
  isEditing: boolean
}

export class GroupHeader extends BaseComponent<GroupHeaderProps, GroupHeaderState> {
  private copyTimeout: number | null = null;

  state = {
    showEditName: false,
    editingName: '',
    showConfirmDelete: false,
    isDeleting: false,
    showEditImage: false,
    editingImage: '',
    isUpdatingImage: false,
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

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }
    if (this.state.showEditImage) {
      this.removeEscapeListener();
    }
  }

  private handleEscapeKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      this.setState({ showEditImage: false });
    }
  }

  private addEscapeListener = () => {
    document.addEventListener('keydown', this.handleEscapeKey);
  }

  private removeEscapeListener = () => {
    document.removeEventListener('keydown', this.handleEscapeKey);
  }

  private handleShowEditImage = () => {
    this.setState({
      showEditImage: true,
      editingImage: this.props.group.picture || ''
    });
    this.addEscapeListener();
  }

  private handleHideEditImage = () => {
    this.setState({ showEditImage: false });
    this.removeEscapeListener();
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

  handleDeleteGroup = async () => {
    this.setState({ isDeleting: true })
    try {
      await this.props.client.deleteGroup(this.props.group.id)
      this.props.showMessage('Group deleted successfully', 'success')
      this.props.onDelete?.(this.props.group.id)
    } catch (error) {
      console.error('Failed to delete group:', error)
      this.showError('Failed to delete group', error)
    } finally {
      this.setState({ isDeleting: false, showConfirmDelete: false })
    }
  }

  handleImageSubmit = async (e: Event) => {
    e.preventDefault();
    if (!this.state.editingImage.trim() || this.state.editingImage === this.props.group.picture) {
      this.handleHideEditImage();
      return;
    }

    this.setState({ isUpdatingImage: true });
    try {
      await this.props.client.updateGroupMetadata({
        ...this.props.group,
        picture: this.state.editingImage
      });
      this.props.group.picture = this.state.editingImage;
      this.handleHideEditImage();
      this.props.showMessage('Group image updated successfully!', 'success');
    } catch (error) {
      console.error('Failed to update group image:', error);
      this.showError('Failed to update group image', error);
      this.handleHideEditImage();
    } finally {
      this.setState({ isUpdatingImage: false });
    }
  }

  handleNameSubmit = async (e: Event) => {
    e.preventDefault();
    const { editingName } = this.state;
    const { group, client, showMessage } = this.props;

    if (!editingName.trim() || editingName === group.name) {
      this.setState({ showEditName: false });
      return;
    }

    try {
      await client.updateGroupMetadata({
        ...group,
        name: editingName
      });
      group.name = editingName;
      this.setState({ showEditName: false });
      showMessage('Group name updated successfully!', 'success');
    } catch (error) {
      console.error('Failed to update group name:', error);
      this.showError('Failed to update group name', error);
    }
  }

  render() {
    const { group, client, showMessage } = this.props
    const { showEditName, editingName, showConfirmDelete, isDeleting, showEditImage, editingImage, isUpdatingImage, isAdmin, isEditing } = this.state

    return (
      <div class="flex-shrink-0">
        <div class="flex items-center justify-between p-4 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
          <div class="flex items-center gap-3">
            <div class="relative group">
              <div
                class="w-10 h-10 bg-[var(--color-bg-primary)] rounded-lg flex items-center justify-center text-lg overflow-hidden cursor-pointer"
                onClick={this.handleShowEditImage}
              >
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
                {/* Hover overlay */}
                <div class="absolute inset-0 bg-black bg-opacity-50 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity">
                  <svg class="w-4 h-4 text-white" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    <path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                </div>
              </div>
              {/* Admin badge */}
              {isAdmin && (
                <div class="absolute -bottom-1 -right-1 rounded-full bg-purple-500 p-1 shadow-lg border-2 border-[var(--color-bg-primary)]">
                  <svg class="w-2.5 h-2.5 text-[var(--color-bg-primary)]" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
                    <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" />
                  </svg>
                </div>
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
                  <div class="flex items-center gap-2">
                    <h2 class="text-lg font-medium text-[var(--color-text-primary)]">{group.name}</h2>
                    {isAdmin && (
                      <span class="shrink-0 px-1.5 py-0.5 text-[10px] font-medium bg-purple-500/10 text-purple-400 rounded-full border border-purple-500/20 flex items-center gap-1">
                        <svg class="w-2.5 h-2.5" viewBox="0 0 24 24" fill="currentColor" xmlns="http://www.w3.org/2000/svg">
                          <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" />
                        </svg>
                        Admin
                      </span>
                    )}
                  </div>
                  <button
                    onClick={() => this.setState({ showEditName: true, editingName: group.name })}
                    class="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
                    title="Edit group name"
                  >
                    <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                      <path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                      <path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
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
                class="text-xs text-red-400 hover:text-red-300 transition-colors
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
                    <svg class="w-3.5 h-3.5 text-red-400" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                      <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                      <path d="M10 11v6M14 11v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  </>
                )}
              </button>
            )
          )}
        </div>

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

        {/* Image Edit Modal */}
        {showEditImage && (
          <div class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center p-4 z-50">
            <div class="bg-[var(--color-bg-secondary)] rounded-lg shadow-xl max-w-md w-full p-4 space-y-4">
              <h3 class="text-lg font-medium text-[var(--color-text-primary)]">Edit Group Image</h3>
              <form onSubmit={this.handleImageSubmit} class="space-y-4">
                <div class="space-y-2">
                  <label class="block text-sm font-medium text-[var(--color-text-secondary)]">
                    Image URL
                  </label>
                  <input
                    type="url"
                    value={editingImage}
                    onInput={(e) => this.setState({ editingImage: (e.target as HTMLInputElement).value })}
                    placeholder="Enter image URL"
                    class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)]
                           text-sm rounded-lg text-[var(--color-text-primary)]
                           placeholder-[var(--color-text-tertiary)]
                           focus:outline-none focus:ring-1 focus:ring-accent
                           hover:border-[var(--color-border-hover)] transition-colors"
                    disabled={isUpdatingImage}
                  />
                </div>

                {/* Preview */}
                {editingImage && (
                  <div class="relative w-20 h-20 mx-auto">
                    <img
                      src={editingImage}
                      alt="Preview"
                      class="w-full h-full object-cover rounded-lg"
                      onError={(e) => {
                        (e.target as HTMLImageElement).style.display = 'none';
                        e.currentTarget.parentElement!.textContent = 'Invalid URL';
                      }}
                    />
                  </div>
                )}

                <div class="flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={this.handleHideEditImage}
                    class="px-4 py-2 text-sm text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                    disabled={isUpdatingImage}
                  >
                    Cancel
                  </button>
                  <button
                    type="submit"
                    disabled={!editingImage.trim() || isUpdatingImage}
                    class="px-4 py-2 bg-accent text-white rounded-lg text-sm font-medium
                           hover:bg-accent/90 disabled:opacity-50 disabled:cursor-not-allowed
                           transition-colors flex items-center gap-2"
                  >
                    {isUpdatingImage ? (
                      <>
                        <span class="animate-spin">⚡</span>
                        Updating...
                      </>
                    ) : (
                      'Save'
                    )}
                  </button>
                </div>
              </form>
            </div>
          </div>
        )}
      </div>
    )
  }
}