import { Component } from 'preact'
import type { Group } from '../types'

interface GroupInfoProps {
  group: Group
  isEditingAbout: boolean
  newAbout: string
  onAboutEdit: () => void
  onAboutSave: () => void
  onAboutChange: (about: string) => void
  onMetadataChange: (field: 'private' | 'closed', value: boolean) => void
}

export class GroupInfo extends Component<GroupInfoProps> {
  render() {
    const {
      group,
      isEditingAbout,
      newAbout,
      onAboutEdit,
      onAboutSave,
      onAboutChange,
      onMetadataChange
    } = this.props

    return (
      <div class="space-y-3">
        <div>
          <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">ID:</span>
          <div class="mt-0.5 text-xs text-[var(--color-text-primary)] break-all">{group.id}</div>
        </div>
        <div>
          <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">About:</span>
          {isEditingAbout ? (
            <div class="mt-0.5 flex items-start gap-2">
              <textarea
                value={newAbout}
                onInput={e => onAboutChange((e.target as HTMLTextAreaElement).value)}
                class="flex-1 rounded border border-[var(--color-border)] px-2 py-1 text-xs
                       bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                       focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                       focus:ring-[var(--color-accent)]/10 transition-all resize-none"
                rows={2}
                autoFocus
              />
              <button
                onClick={onAboutSave}
                class="px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                       hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                       transition-all disabled:opacity-50"
              >
                Save
              </button>
            </div>
          ) : (
            <div
              class="mt-0.5 text-xs text-[var(--color-text-primary)] cursor-pointer group
                     hover:bg-[var(--color-bg-tertiary)] transition-colors rounded px-2 py-1 -mx-2
                     flex items-center gap-1"
              onClick={onAboutEdit}
            >
              {group.about || "No description"}
              <span class="text-xs text-[var(--color-text-secondary)] opacity-0 group-hover:opacity-100 transition-opacity">
                ✏️ edit
              </span>
            </div>
          )}
        </div>
        <div>
          <span class="text-xs font-medium text-[var(--color-text-secondary)] uppercase tracking-wide">Type:</span>
          <div class="mt-1 flex gap-3">
            <label class="flex items-center gap-1.5 cursor-pointer">
              <input
                type="checkbox"
                checked={group.private}
                onChange={() => onMetadataChange('private', !group.private)}
                class="w-3 h-3 rounded border-[var(--color-border)] text-[var(--color-accent)]
                       focus:ring-[var(--color-accent)] cursor-pointer bg-[var(--color-bg-tertiary)]"
              />
              <span class="text-xs text-[var(--color-text-primary)]">Private</span>
            </label>
            <label class="flex items-center gap-1.5 cursor-pointer">
              <input
                type="checkbox"
                checked={group.closed}
                onChange={() => onMetadataChange('closed', !group.closed)}
                class="w-3 h-3 rounded border-[var(--color-border)] text-[var(--color-accent)]
                       focus:ring-[var(--color-accent)] cursor-pointer bg-[var(--color-bg-tertiary)]"
              />
              <span class="text-xs text-[var(--color-text-primary)]">Closed</span>
            </label>
          </div>
        </div>
      </div>
    )
  }
}