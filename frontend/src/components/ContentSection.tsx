import type { Group } from '../types'

interface ContentSectionProps {
  group: Group
}

export function ContentSection({ group }: ContentSectionProps) {
  if (!group.content?.length) {
    return null
  }

  return (
    <section class="border-t border-[var(--color-border)] flex-1 flex flex-col min-h-0">
      <h3 class="flex items-center gap-1 text-sm font-semibold text-[var(--color-text-primary)] p-3 border-b border-[var(--color-border)] bg-gradient-to-r from-[var(--color-bg-tertiary)] to-[var(--color-bg-secondary)]">
        <span class="text-base">💬</span> Recent Activity
      </h3>

      <div class="p-4 overflow-y-auto flex-1">
        <ul class="space-y-4 max-w-4xl mx-auto">
          {group.content.map((item, index) => (
            <li key={index} class="rounded-lg bg-[var(--color-bg-tertiary)] p-4 hover:bg-[var(--color-bg-tertiary)]/80 transition-colors">
              <div class="space-y-3">
                <div class="flex items-center justify-between gap-2">
                  <div class="flex items-center gap-2">
                    <div class="w-8 h-8 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-white text-xs font-medium">
                      {item.pubkey.slice(0, 2).toUpperCase()}
                    </div>
                    <div class="text-xs font-mono text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors cursor-pointer" data-tooltip={item.pubkey}>
                      {item.pubkey.slice(0, 8)}...
                    </div>
                  </div>
                  <span class="text-xs text-[var(--color-text-secondary)]">
                    {new Date(item.created_at * 1000).toLocaleString()}
                  </span>
                </div>
                <div class="text-sm text-[var(--color-text-primary)] break-words leading-relaxed">
                  {item.content}
                </div>
              </div>
            </li>
          ))}
        </ul>
      </div>
    </section>
  )
}