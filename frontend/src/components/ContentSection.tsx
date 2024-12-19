import { FunctionComponent } from 'preact'
import { NostrClient } from '../api/nostr_client'
import type { Group } from '../types'
import { PubkeyDisplay } from './PubkeyDisplay'

interface ContentSectionProps {
  group: Group
  client: NostrClient
}

export const ContentSection: FunctionComponent<ContentSectionProps> = ({ group }) => {
  const formatTimestamp = (timestamp: number) => {
    const date = new Date(timestamp * 1000)
    return date.toLocaleString()
  }

  const content = group.content || []

  return (
    <div class="h-full flex flex-col">
      <div class="p-4 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
        <h3 class="text-sm font-medium text-[var(--color-text-primary)] flex items-center gap-2">
          <span>💬</span>
          Recent Activity
        </h3>
      </div>

      <div class="flex-1 overflow-y-auto p-4">
        <div class="space-y-4">
          {content.map((item, index) => (
            <div
              key={index}
              class="p-3 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]
                     hover:border-[var(--color-border-hover)] transition-colors"
            >
              <div class="flex items-start justify-between gap-4">
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2 mb-2">
                    <PubkeyDisplay pubkey={item.pubkey} showCopy={false} />
                    <span class="text-xs text-[var(--color-text-tertiary)]">
                      {formatTimestamp(item.created_at)}
                    </span>
                  </div>
                  <p class="text-sm text-[var(--color-text-primary)] break-words leading-relaxed">
                    {item.content}
                  </p>
                </div>
              </div>
            </div>
          ))}

          {content.length === 0 && (
            <div class="text-center py-12">
              <div class="mb-3 text-2xl">💭</div>
              <p class="text-sm text-[var(--color-text-tertiary)]">No activity yet</p>
              <p class="text-xs text-[var(--color-text-tertiary)] mt-1">
                Messages will appear here when members start posting
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}