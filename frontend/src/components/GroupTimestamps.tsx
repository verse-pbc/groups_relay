import { FunctionComponent } from 'preact'
import type { Group } from '../types'

interface GroupTimestampsProps {
  group: Group
}

export const GroupTimestamps: FunctionComponent<GroupTimestampsProps> = ({ group }) => {
  const formatDate = (timestamp: number) => {
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

    // Format the date
    const options: Intl.DateTimeFormatOptions = {
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: 'numeric',
      hour12: true
    }

    // Add year if it's not current year
    if (date.getFullYear() !== now.getFullYear()) {
      options.year = 'numeric'
    }

    return date.toLocaleString(undefined, options)
  }

  return (
    <div class="space-y-2">
      <div class="flex items-center gap-2 text-sm text-[var(--color-text-tertiary)]">
        <span>⏱️</span>
        <div class="space-y-1">
          <div class="flex items-center gap-1">
            <span class="text-xs font-medium">Created:</span>
            <span class="text-xs">{formatDate(group.created_at)}</span>
          </div>
          <div class="flex items-center gap-1">
            <span class="text-xs font-medium">Updated:</span>
            <span class="text-xs">{formatDate(group.updated_at)}</span>
          </div>
        </div>
      </div>
    </div>
  )
}