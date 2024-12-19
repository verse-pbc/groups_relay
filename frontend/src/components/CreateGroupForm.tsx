import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import { Group } from '../types'
import { LogoutButton } from './LogoutButton'

interface CreateGroupFormProps {
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
  onLogout: () => void
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

export class CreateGroupForm extends Component<CreateGroupFormProps, CreateGroupFormState> {
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
        created_at: Math.floor(Date.now() / 1000),
        updated_at: Math.floor(Date.now() / 1000),
        members: [],
        invites: {},
        joinRequests: [],
        content: [],
      })

      this.props.updateGroupsMap(groupsMap => {
        groupsMap.set(group.id, group)
      })

      this.setState({
        groupId: generateGroupId(),
        name: '',
        about: '',
        picture: '',
      })

      this.props.showMessage('Group created successfully!', 'success')
    } catch (error) {
      console.error('Failed to create group:', error)
      this.props.showMessage('Failed to create group: ' + error, 'error')
    } finally {
      this.setState({ isSubmitting: false })
    }
  }

  render() {
    const { isSubmitting } = this.state
    const { onLogout } = this.props

    return (
      <div class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] p-4">
        <h2 class="text-lg font-semibold text-[var(--color-text-primary)] mb-4">Create New Group</h2>
        <form onSubmit={this.handleSubmit} class="space-y-4">
          <div>
            <label class="block text-sm font-medium text-[var(--color-text-secondary)] mb-1">
              Name
            </label>
            <input
              type="text"
              value={this.state.name}
              onChange={(e) => this.setState({ name: (e.target as HTMLInputElement).value })}
              placeholder="Enter group name"
              class="w-full p-2 border border-[var(--color-border)] rounded bg-[var(--color-bg-primary)]
                     text-[var(--color-text-primary)] placeholder-[var(--color-text-tertiary)]
                     focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
              required
            />
          </div>

          <div>
            <label class="block text-sm font-medium text-[var(--color-text-secondary)] mb-1">
              Description
            </label>
            <textarea
              value={this.state.about}
              onChange={(e) => this.setState({ about: (e.target as HTMLTextAreaElement).value })}
              placeholder="Enter group description"
              class="w-full p-2 border border-[var(--color-border)] rounded bg-[var(--color-bg-primary)]
                     text-[var(--color-text-primary)] placeholder-[var(--color-text-tertiary)]
                     focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
              rows={3}
            />
          </div>

          <div>
            <label class="block text-sm font-medium text-[var(--color-text-secondary)] mb-1">
              Picture URL
            </label>
            <input
              type="url"
              value={this.state.picture}
              onChange={(e) => this.setState({ picture: (e.target as HTMLInputElement).value })}
              placeholder="Enter picture URL"
              class="w-full p-2 border border-[var(--color-border)] rounded bg-[var(--color-bg-primary)]
                     text-[var(--color-text-primary)] placeholder-[var(--color-text-tertiary)]
                     focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]"
            />
          </div>

          <button
            type="submit"
            disabled={isSubmitting}
            class="w-full px-4 py-2 bg-[var(--color-accent)] text-white rounded
                   hover:bg-[var(--color-accent-hover)] disabled:opacity-50 disabled:cursor-not-allowed
                   transition-colors flex items-center justify-center gap-2"
          >
            {isSubmitting ? (
              <>
                <span class="animate-spin">⚡</span>
                Creating...
              </>
            ) : (
              'Create Group'
            )}
          </button>
        </form>

        <LogoutButton onLogout={onLogout} />
      </div>
    )
  }
}