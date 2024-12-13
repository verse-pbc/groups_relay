import type { Group } from '../types'

interface ContentSectionProps {
  group: Group
}

export function ContentSection({ group }: ContentSectionProps) {
  if (!group.content?.length) {
    return null
  }

  return (
    <section class="border-t border-gray-200 p-3">
      <h3 class="flex items-center gap-1 text-sm font-semibold text-gray-900 mb-2">
        <span class="text-base">💬</span> Recent Activity
      </h3>

      <ul class="space-y-2">
        {group.content.map((item, index) => (
          <li key={index} class="py-1">
            <div class="space-y-1">
              <div class="flex items-center gap-2">
                <div class="text-xs text-gray-500">{item.pubkey}</div>
                <span class="text-xs text-gray-400">
                  {new Date(item.created_at * 1000).toLocaleString()}
                </span>
              </div>
              <div class="text-xs text-gray-900">{item.content}</div>
            </div>
          </li>
        ))}
      </ul>
    </section>
  )
}