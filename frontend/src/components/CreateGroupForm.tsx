import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import { Group } from '../types'

interface CreateGroupFormProps {
  updateGroupsMap: (updater: (map: Map<string, Group>) => void) => void
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
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
    this.setState({ isSubmitting: true })

    try {
      await this.props.client.createGroup(
        this.state.groupId,
        this.state.name,
        this.state.about,
        this.state.picture
      )
      this.props.showMessage('Group created successfully!', 'success')
      this.setState({
        groupId: generateGroupId(),
        name: '',
        about: '',
        picture: '',
      })
    } catch (error) {
      console.error('Failed to create group:', error)
      this.props.showMessage('Failed to create group: ' + error, 'error')
    } finally {
      this.setState({ isSubmitting: false })
    }
  }

  render() {
    const { groupId, name, about, picture, isSubmitting } = this.state

    return (
      <form onSubmit={this.handleSubmit} class="bg-[var(--color-bg-secondary)] rounded-lg shadow-lg border border-[var(--color-border)] p-2 w-full">
        <h2 class="text-sm font-semibold text-[var(--color-text-primary)] mb-2">Create New Group</h2>

        <div class="space-y-1.5">
          <div>
            <label htmlFor="groupId" class="block text-xs font-medium text-[var(--color-text-secondary)] mb-0.5">
              Group ID
            </label>
            <input
              type="text"
              id="groupId"
              value={groupId}
              class="block w-full rounded border border-[var(--color-border)] px-2 py-1 text-xs
                     bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                     focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                     focus:ring-[var(--color-accent)]/10 transition-all font-mono"
              disabled
            />
          </div>

          <div>
            <label htmlFor="name" class="block text-xs font-medium text-[var(--color-text-secondary)] mb-0.5">
              Name *
            </label>
            <input
              type="text"
              id="name"
              value={name}
              onInput={e => this.setState({ name: (e.target as HTMLInputElement).value })}
              class="block w-full rounded border border-[var(--color-border)] px-2 py-1 text-xs
                     bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                     focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                     focus:ring-[var(--color-accent)]/10 transition-all"
              required
            />
          </div>

          <div>
            <label htmlFor="about" class="block text-xs font-medium text-[var(--color-text-secondary)] mb-0.5">
              About
            </label>
            <textarea
              id="about"
              value={about}
              onInput={e => this.setState({ about: (e.target as HTMLTextAreaElement).value })}
              rows={2}
              class="block w-full rounded border border-[var(--color-border)] px-2 py-1 text-xs
                     bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                     focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                     focus:ring-[var(--color-accent)]/10 transition-all"
            />
          </div>

          <div>
            <label htmlFor="picture" class="block text-xs font-medium text-[var(--color-text-secondary)] mb-0.5">
              Picture URL
            </label>
            <input
              type="url"
              id="picture"
              value={picture}
              onInput={e => this.setState({ picture: (e.target as HTMLInputElement).value })}
              class="block w-full rounded border border-[var(--color-border)] px-2 py-1 text-xs
                     bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                     focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                     focus:ring-[var(--color-accent)]/10 transition-all"
            />
          </div>

          <button
            type="submit"
            disabled={isSubmitting || !name.trim()}
            class="w-full mt-1.5 flex justify-center py-1 px-2 border border-transparent rounded text-xs
                   font-medium text-white bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)]
                   focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-[var(--color-accent)]
                   disabled:opacity-50 transition-all"
          >
            {isSubmitting ? 'Creating...' : 'Create Group'}
          </button>
        </div>
      </form>
    )
  }
}