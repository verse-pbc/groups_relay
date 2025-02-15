import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { UserDisplay } from './UserDisplay'
import { BaseComponent } from './BaseComponent'

interface ContentSectionProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

interface ContentSectionState {
  deletingEvents: Set<string>
  showConfirmDelete: string | null
}

export class ContentSection extends BaseComponent<ContentSectionProps, ContentSectionState> {
  state = {
    deletingEvents: new Set<string>(),
    showConfirmDelete: null
  }

  handleDeleteEvent = async (eventId: string) => {
    this.setState(prev => ({
      deletingEvents: new Set(prev.deletingEvents).add(eventId),
      showConfirmDelete: null
    }))

    try {
      await this.props.client.deleteEvent(this.props.group.id, eventId)
      this.props.group.content = this.props.group.content?.filter(item => item.id !== eventId) || []
      this.props.showMessage('Event deleted successfully', 'success')
    } catch (error) {
      console.error('Failed to delete event:', error)
      this.showError('Failed to delete event', error)
    } finally {
      this.setState(prev => {
        const newSet = new Set(prev.deletingEvents)
        newSet.delete(eventId)
        return { deletingEvents: newSet }
      })
    }
  }

  formatTimestamp = (timestamp: number) => {
    const date = new Date(timestamp * 1000)
    const now = new Date()
    const diffInSeconds = Math.floor((now.getTime() - date.getTime()) / 1000)

    // Less than a minute ago
    if (diffInSeconds < 60) {
      return 'just now'
    }

    // Less than an hour ago
    if (diffInSeconds < 3600) {
      const minutes = Math.floor(diffInSeconds / 60)
      return `${minutes}m ago`
    }

    // Less than a day ago
    if (diffInSeconds < 86400) {
      const hours = Math.floor(diffInSeconds / 3600)
      return `${hours}h ago`
    }

    // Less than a week ago
    if (diffInSeconds < 604800) {
      const days = Math.floor(diffInSeconds / 86400)
      return `${days}d ago`
    }

    // If it's this year
    if (date.getFullYear() === now.getFullYear()) {
      return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
    }

    // If it's a different year
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
  }

  render() {
    const { group } = this.props
    const { deletingEvents, showConfirmDelete } = this.state
    const content = group.content || []

    return (
      <div class="h-full flex flex-col overflow-hidden">
        <div class="flex-1 overflow-y-auto">
          <div class="space-y-3">
            {content.map((item) => (
              <div
                key={item.id}
                class="group p-2 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]
                       hover:border-[var(--color-border-hover)] transition-colors relative"
              >
                <div class="flex items-start gap-1.5">
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center text-[11px] gap-1.5 text-[var(--color-text-tertiary)]">
                      <UserDisplay
                        pubkey={this.props.client.pubkeyToNpub(item.pubkey)}
                        client={this.props.client}
                        showCopy={true}
                        size="sm"
                        onCopy={() => this.props.showMessage('Npub copied to clipboard', 'success')}
                      />
                      <span>·</span>
                      <span>
                        {this.formatTimestamp(item.created_at)}
                      </span>
                    </div>
                    <p class="text-sm text-[var(--color-text-primary)] break-all whitespace-pre-wrap leading-relaxed mt-0.5">
                      {item.content}
                    </p>
                  </div>

                  {showConfirmDelete === item.id ? (
                    <div class="flex items-center gap-1 text-[11px]">
                      <button
                        onClick={() => this.handleDeleteEvent(item.id)}
                        class="text-red-400 hover:text-red-300 transition-colors"
                      >
                        Delete
                      </button>
                      <span class="text-[var(--color-text-tertiary)]">·</span>
                      <button
                        onClick={() => this.setState({ showConfirmDelete: null })}
                        class="text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                      >
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <button
                      onClick={() => this.setState({ showConfirmDelete: item.id })}
                      disabled={deletingEvents.has(item.id)}
                      class="text-[11px] opacity-0 group-hover:opacity-100 text-red-400
                             hover:text-red-300 transition-all duration-150 flex items-center"
                      title="Delete message"
                    >
                      {deletingEvents.has(item.id) ? (
                        <span class="animate-spin">⚡</span>
                      ) : (
                        <svg class="w-3.5 h-3.5 text-red-400" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                          <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                          <path d="M10 11v6M14 11v6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                        </svg>
                      )}
                    </button>
                  )}
                </div>
              </div>
            ))}

            {content.length === 0 && (
              <div class="text-center py-12">
                <div class="mb-3 text-2xl">💭</div>
                <p class="text-sm text-[#8484ac]">No activity yet</p>
                <p class="text-xs text-[#8484ac] mt-1">
                  Messages will appear here when members start posting
                </p>
              </div>
            )}
          </div>
        </div>
      </div>
    )
  }
}