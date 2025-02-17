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
  onDelete?: (groupId: string) => void
}

interface GroupInfoState {
  editingName: string
  editingAbout: string
  showConfirmDelete: boolean
  isDeleting: boolean
  showEditName: boolean
  showEditAbout: boolean
  showEditImage: boolean
  editingImage: string
  isUpdatingImage: boolean
  isAdmin: boolean
  isMember: boolean
}

export class GroupInfo extends BaseComponent<GroupInfoProps, GroupInfoState> {
  state = {
    editingName: '',
    editingAbout: '',
    showConfirmDelete: false,
    isDeleting: false,
    showEditName: false,
    showEditAbout: false,
    showEditImage: false,
    editingImage: '',
    isUpdatingImage: false,
    isAdmin: false,
    isMember: false
  }

  async componentDidMount() {
    const user = await this.props.client.ndkInstance.signer?.user();
    if (user?.pubkey) {
      const member = this.props.group.members.find(m => m.pubkey === user.pubkey);
      const isAdmin = member?.roles.includes('Admin') || false;
      const isMember = !!member;
      this.setState({ isAdmin, isMember });
    }
  }

  async componentDidUpdate(prevProps: GroupInfoProps) {
    if (prevProps.group.id !== this.props.group.id) {
      const user = await this.props.client.ndkInstance.signer?.user();
      if (user?.pubkey) {
        const member = this.props.group.members.find(m => m.pubkey === user.pubkey);
        const isAdmin = member?.roles.includes('Admin') || false;
        const isMember = !!member;
        this.setState({ isAdmin, isMember });
      }
    }
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

  componentWillUnmount() {
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

  handleAboutSubmit = async (e: Event) => {
    e.preventDefault();
    const { editingAbout } = this.state;
    const { group, client, showMessage } = this.props;

    if (editingAbout === group.about) {
      this.setState({ showEditAbout: false });
      return;
    }

    try {
      await client.updateGroupMetadata({
        ...group,
        about: editingAbout
      });
      group.about = editingAbout;
      this.setState({ showEditAbout: false });
      showMessage('Group description updated successfully!', 'success');
    } catch (error) {
      console.error('Failed to update group description:', error);
      this.showError('Failed to update group description', error);
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

  render() {
    const { group, isEditing, onEditCancel } = this.props
    const { editingName, editingAbout, showConfirmDelete, isDeleting, showEditName, showEditAbout, showEditImage, editingImage, isUpdatingImage } = this.state

    return (
      <div class="space-y-6">
        {/* Group Avatar and Name/About */}
        <div class="flex flex-col gap-4">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-4">
              <div
                class="relative w-20 h-20 bg-[var(--color-bg-primary)] rounded-full flex items-center justify-center text-3xl overflow-hidden border border-[var(--color-border)] group cursor-pointer"
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
                  <svg class="w-6 h-6 text-white" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    <path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  </svg>
                </div>
              </div>
              {!isEditing && (
                <div class="flex-1 space-y-1">
                  {showEditName ? (
                    <form onSubmit={this.handleNameSubmit} class="flex items-center gap-2">
                      <input
                        type="text"
                        value={editingName}
                        onInput={(e: Event) => this.setState({ editingName: (e.target as HTMLInputElement).value })}
                        class="px-2 py-1 text-2xl bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded"
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
                    <div class="flex items-center gap-2 group">
                      <div class="flex items-center gap-2">
                        <h2 class="text-3xl font-medium text-[var(--color-text-primary)]">{group.name}</h2>
                        {this.state.isAdmin ? (
                          <span class="shrink-0 px-2 py-0.5 text-xs font-medium bg-purple-500/10 text-purple-400 rounded-full border border-purple-500/20">
                            Admin
                          </span>
                        ) : this.state.isMember && (
                          <span class="shrink-0 px-2 py-0.5 text-xs font-medium bg-blue-500/10 text-blue-400 rounded-full border border-blue-500/20">
                            Member
                          </span>
                        )}
                      </div>
                      <button
                        onClick={() => this.setState({ showEditName: true, editingName: group.name })}
                        class="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors opacity-0 group-hover:opacity-100"
                        title="Edit group name"
                      >
                        <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                          <path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                          <path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                        </svg>
                      </button>
                    </div>
                  )}
                  {showEditAbout ? (
                    <form onSubmit={this.handleAboutSubmit} class="flex items-center gap-2">
                      <input
                        type="text"
                        value={editingAbout}
                        onInput={(e: Event) => this.setState({ editingAbout: (e.target as HTMLInputElement).value })}
                        class="px-2 py-1 text-sm bg-[var(--color-bg-primary)] border border-[var(--color-border)] rounded w-full"
                        placeholder="Enter group description"
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
                          onClick={() => this.setState({ showEditAbout: false })}
                          class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                        >
                          Cancel
                        </button>
                      </div>
                    </form>
                  ) : (
                    <div class="flex items-center gap-2 group">
                      <p class="text-sm text-[var(--color-text-secondary)]">
                        {group.about || 'No description'}
                      </p>
                      <button
                        onClick={() => this.setState({ showEditAbout: true, editingAbout: group.about || '' })}
                        class="text-sm text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors opacity-0 group-hover:opacity-100"
                        title="Edit group description"
                      >
                        <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                          <path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                          <path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                        </svg>
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* Delete Group Button */}
            {!isEditing && (
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
                  class="text-red-400 hover:text-red-300 transition-colors"
                  title="Delete group"
                >
                  {isDeleting ? (
                    <span class="animate-spin">⚡</span>
                  ) : (
                    <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                      <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                      <path d="M10 11v6M14 11v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  )}
                </button>
              )
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