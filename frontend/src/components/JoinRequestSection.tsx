import { Component } from 'preact'
import { NostrClient, NostrGroupError } from '../api/nostr_client'
import type { Group } from '../types'
import { JoinRequestForm } from './JoinRequestForm'
import { PubkeyDisplay } from './PubkeyDisplay'

interface JoinRequestSectionProps {
  group: Group
  client: NostrClient
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

interface JoinRequestSectionState {
  showJoinForm: boolean
  inviteCode: string
  isSubmitting: boolean
}

export class JoinRequestSection extends Component<JoinRequestSectionProps, JoinRequestSectionState> {
  state = {
    showJoinForm: false,
    inviteCode: '',
    isSubmitting: false
  }

  private showError = (prefix: string, error: unknown) => {
    console.error(prefix, error)
    const message = error instanceof NostrGroupError ? error.displayMessage : String(error)
    this.props.showMessage(`${prefix}: ${message}`, 'error')
  }

  handleAcceptRequest = async (pubkey: string) => {
    try {
      await this.props.client.acceptJoinRequest(this.props.group.id, pubkey)
      this.props.showMessage('Join request accepted successfully', 'success')
    } catch (error) {
      this.showError('Failed to accept join request', error)
    }
  }

  handleRejectRequest = async (pubkey: string) => {
    try {
      await this.props.client.deleteEvent(this.props.group.id, pubkey)
      this.props.showMessage('Join request rejected successfully', 'success')
    } catch (error) {
      this.showError('Failed to reject join request', error)
    }
  }

  handleSubmitJoinRequest = async (e: Event) => {
    e.preventDefault()
    if (!this.state.inviteCode.trim()) return

    this.setState({ isSubmitting: true })
    try {
      await this.props.client.sendJoinRequest(this.props.group.id, this.state.inviteCode)
      this.setState({ inviteCode: '', showJoinForm: false })
      this.props.showMessage('Join request submitted successfully', 'success')
    } catch (error) {
      this.showError('Failed to submit join request', error)
    } finally {
      this.setState({ isSubmitting: false })
    }
  }

  render() {
    const { group, client } = this.props

    return (
      <div class="space-y-4">
        <div class="p-4 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
          <JoinRequestForm
            groupId={group.id}
            relayUrl={client.config.relayUrl}
            client={client}
          />
        </div>

        {group.joinRequests.length > 0 ? (
          <div class="space-y-2">
            {group.joinRequests.map(pubkey => (
              <div
                key={pubkey}
                class="flex items-center justify-between gap-2 p-4 bg-[var(--color-bg-primary)]
                       rounded-lg border border-[var(--color-border)] hover:border-[var(--color-border-hover)]
                       transition-colors"
              >
                <PubkeyDisplay pubkey={pubkey} showCopy={false} />
                <button
                  onClick={() => this.handleAcceptRequest(pubkey)}
                  class="shrink-0 px-4 py-2 bg-accent text-white rounded-lg text-sm font-medium
                         hover:bg-accent/90 transition-colors flex items-center gap-2"
                >
                  Accept
                </button>
              </div>
            ))}
          </div>
        ) : (
          <div class="text-center py-12">
            <div class="mb-3 text-2xl">🤝</div>
            <p class="text-sm text-[var(--color-text-tertiary)]">No pending join requests</p>
            <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
              Share the group invite code to let others join
            </p>
          </div>
        )}
      </div>
    )
  }
}