import { NostrClient } from '../api/nostr_client'
import { Group } from '../types'
import { BaseComponent } from './BaseComponent'

interface CreateGroupFormProps {
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onGroupCreated?: (group: Group) => void
}

interface CreateGroupFormState {
  groupId: string
  name: string
  about: string
  picture: string
  isSubmitting: boolean
}

function generateGroupId(): string {
  const chars = 'abcdefghijklmnopqrstuvwxyz0123456789'
  return Array.from(
    { length: 12 },
    () => chars[Math.floor(Math.random() * chars.length)]
  ).join('')
}

export class CreateGroupForm extends BaseComponent<CreateGroupFormProps, CreateGroupFormState> {
  constructor(props: CreateGroupFormProps) {
    super(props)
    this.state = {
      groupId: generateGroupId(),
      name: '',
      about: '',
      picture: '',
      isSubmitting: false
    }
  }

  handleSubmit = async (e: Event) => {
    e.preventDefault()
    if (!this.state.name.trim()) return

    this.setState({ isSubmitting: true })
    try {
      const group = await this.props.client.createGroup({
        id: this.state.groupId,
        name: this.state.name,
        about: this.state.about,
        picture: this.state.picture,
        private: false,
        closed: false,
        broadcast: false,
        created_at: Math.floor(Date.now() / 1000),
        updated_at: Math.floor(Date.now() / 1000),
        members: [],
        invites: {},
        joinRequests: [],
        content: [],
      })

      this.props.updateGroupsMap(groupsMap => {
        // Create a deep copy of the group to prevent reference issues
        const groupCopy = {
          ...group,
          members: [...group.members],
          joinRequests: [...group.joinRequests],
          invites: { ...group.invites },
          content: group.content ? [...group.content] : []
        };

        // Get existing group if any
        const existingGroup = groupsMap.get(group.id);
        if (existingGroup) {
          // Preserve existing members and state
          groupCopy.members = [...existingGroup.members];
          groupCopy.joinRequests = [...existingGroup.joinRequests];
          groupCopy.invites = { ...existingGroup.invites };
          groupCopy.content = existingGroup.content ? [...existingGroup.content] : [];
        }

        groupsMap.set(group.id, groupCopy);
      })

      // Queue the group for selection
      if (this.props.onGroupCreated) {
        this.props.onGroupCreated(group);
      }

      this.setState({
        groupId: generateGroupId(),
        name: '',
        about: '',
        picture: '',
      })

      this.props.showMessage('Group created successfully!', 'success')
    } catch (error) {
      console.error('Failed to create group:', error)
      this.showError('Failed to create group', error)
    } finally {
      this.setState({ isSubmitting: false })
    }
  }

  render() {
    const { isSubmitting } = this.state

    return (
      <div class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] p-4">
        <h2 class="text-lg font-semibold text-[var(--color-text-primary)] mb-4">Create New Group</h2>
        <form onSubmit={this.handleSubmit} class="space-y-4">
          <div class="space-y-4">
            <div class="space-y-3">
              <label class="block text-sm font-medium text-white">
                Name
              </label>
              <input
                type="text"
                value={this.state.name}
                onInput={(e) => this.setState({ name: (e.target as HTMLInputElement).value })}
                placeholder="Enter group name"
                class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                       text-sm rounded-lg text-[var(--color-text-primary)]
                       placeholder-[#8484ac]
                       focus:outline-none focus:ring-1 focus:ring-accent
                       hover:border-[var(--color-border-hover)] transition-colors"
                required
                disabled={isSubmitting}
              />
            </div>

            <div class="space-y-3">
              <label class="block text-sm font-medium text-white">
                Description
              </label>
              <textarea
                value={this.state.about}
                onInput={(e) => this.setState({ about: (e.target as HTMLTextAreaElement).value })}
                placeholder="Enter group description"
                rows={3}
                class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                       text-sm rounded-lg text-[var(--color-text-primary)]
                       placeholder-[#8484ac]
                       focus:outline-none focus:ring-1 focus:ring-accent
                       hover:border-[var(--color-border-hover)] transition-colors resize-none"
                disabled={isSubmitting}
              />
            </div>

            <div class="space-y-3">
              <label class="block text-sm font-medium text-white">
                Picture URL
              </label>
              <input
                type="url"
                value={this.state.picture}
                onInput={(e) => this.setState({ picture: (e.target as HTMLInputElement).value })}
                placeholder="Enter picture URL"
                class="w-full px-3 py-2 bg-[var(--color-bg-secondary)] border border-[var(--color-border)]
                       text-sm rounded-lg text-[var(--color-text-primary)]
                       placeholder-[#8484ac]
                       focus:outline-none focus:ring-1 focus:ring-accent
                       hover:border-[var(--color-border-hover)] transition-colors"
                disabled={isSubmitting}
              />
            </div>

            <div class="pt-2">
              <button
                type="submit"
                disabled={isSubmitting || !this.state.name.trim()}
                class="w-full px-4 py-2 bg-accent text-white rounded-lg text-sm font-medium
                       hover:bg-accent/90 disabled:opacity-50 disabled:cursor-not-allowed
                       transition-colors flex items-center justify-center gap-2"
              >
                {isSubmitting ? (
                  <>
                    <span class="animate-spin">⚡</span>
                    Creating...
                  </>
                ) : (
                  <>
                    <span>✨</span>
                    Create Group
                  </>
                )}
              </button>
            </div>
          </div>
        </form>
      </div>
    )
  }
}