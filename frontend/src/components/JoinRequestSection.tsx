import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { JoinRequestForm } from './JoinRequestForm'

interface JoinRequestSectionProps {
  group: Group
  client: NostrClient
}

interface JoinRequestSectionState {
  showJoinForm: boolean
}

export class JoinRequestSection extends Component<JoinRequestSectionProps, JoinRequestSectionState> {
  constructor(props: JoinRequestSectionProps) {
    super(props)
    this.state = {
      showJoinForm: true
    }
  }

  handleAcceptRequest = async (pubkey: string) => {
    try {
      await this.props.client.acceptJoinRequest(this.props.group.id, pubkey)
    } catch (error) {
      console.error('Failed to accept join request:', error)
    }
  }

  render() {
    const { group, client } = this.props

    const pendingRequests = group.join_requests?.filter(
      pubkey => !group.members.some(member => member.pubkey === pubkey)
    ) || []

    const truncatePubkey = (pubkey: string) => {
      return pubkey.slice(0, 8) + '...'
    }

    return (
      <section class="border-t border-[var(--color-border)] p-3">
        <h3 class="flex items-center gap-1 text-sm font-semibold text-[var(--color-text-primary)] mb-2">
          <span class="text-base">🤝</span> Join Requests
        </h3>

        <div class="mb-3 border border-[var(--color-border)] rounded p-2 bg-[var(--color-bg-tertiary)]">
          <JoinRequestForm groupId={group.id} relayUrl={client.config.relayUrl} />
        </div>

        {pendingRequests.length > 0 ? (
          <ul class="space-y-2">
            {pendingRequests.map(pubkey => (
              <li key={pubkey} class="py-1">
                <div class="flex items-center justify-between gap-2">
                  <div
                    class="text-xs text-[var(--color-text-secondary)] font-mono hover:text-[var(--color-text-primary)] transition-colors"
                    data-tooltip={pubkey}
                  >
                    {truncatePubkey(pubkey)}
                  </div>
                  <button
                    onClick={() => this.handleAcceptRequest(pubkey)}
                    class="px-2 py-1 bg-[var(--color-accent)] text-white text-xs rounded
                           hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                           transition-all flex-shrink-0"
                  >
                    Accept
                  </button>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <p class="text-xs text-[var(--color-text-secondary)]">No pending join requests.</p>
        )}
      </section>
    )
  }
}