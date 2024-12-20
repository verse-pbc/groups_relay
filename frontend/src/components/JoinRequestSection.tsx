import { Component } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { JoinRequestForm } from './JoinRequestForm'
import { PubkeyDisplay } from './PubkeyDisplay'

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

    const pendingRequests = group.joinRequests?.filter(
      pubkey => !group.members.some(member => member.pubkey === pubkey)
    ) || []

    return (
      <div class="space-y-4">
        <div class="p-4 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
          <JoinRequestForm
            groupId={group.id}
            relayUrl={client.config.relayUrl}
            client={client}
          />
        </div>

        {pendingRequests.length > 0 ? (
          <div class="space-y-2">
            {pendingRequests.map(pubkey => (
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