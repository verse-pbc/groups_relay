import { Component } from 'preact'
import type { Group } from '../types'

interface GroupHeaderProps {
  group: Group
  isEditingName: boolean
  newName: string
  onNameEdit: () => void
  onNameSave: () => void
  onNameChange: (name: string) => void
}

export class GroupHeader extends Component<GroupHeaderProps> {
  render() {
    const { group, isEditingName, newName, onNameEdit, onNameSave, onNameChange } = this.props

    return (
      <header class="p-3 border-b border-[var(--color-border)] bg-gradient-to-r from-[var(--color-bg-tertiary)] to-[var(--color-bg-secondary)]">
        <div class="flex items-center gap-3">
          {group.picture && (
            <img
              src={group.picture}
              alt={group.name}
              class="w-8 h-8 rounded object-cover flex-shrink-0"
              onError={(e) => {
                (e.target as HTMLImageElement).style.display = 'none'
              }}
            />
          )}
          <div class="flex-grow min-w-0">
            {isEditingName ? (
              <div class="flex items-center gap-2">
                <input
                  type="text"
                  value={newName}
                  onInput={e => onNameChange((e.target as HTMLInputElement).value)}
                  class="flex-1 rounded border border-[var(--color-border)] px-2 py-1 text-xs
                         bg-[var(--color-bg-tertiary)] text-[var(--color-text-primary)]
                         focus:border-[var(--color-accent)] focus:outline-none focus:ring-1
                         focus:ring-[var(--color-accent)]/10 transition-all"
                  autoFocus
                />
                <button
                  onClick={onNameSave}
                  class="px-2 py-1 bg-[var(--color-accent)] text-white rounded text-xs font-medium
                         hover:bg-[var(--color-accent-hover)] active:transform active:translate-y-0.5
                         transition-all disabled:opacity-50"
                >
                  Save
                </button>
              </div>
            ) : (
              <h2
                class="text-base font-semibold text-[var(--color-text-primary)] cursor-pointer px-2 py-1
                       rounded group hover:bg-[var(--color-bg-tertiary)] transition-colors flex items-center gap-1
                       truncate"
                onClick={onNameEdit}
              >
                <span class="truncate">{group.name}</span>
                <span class="text-xs text-[var(--color-text-secondary)] opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
                  ✏️ edit
                </span>
              </h2>
            )}
          </div>
        </div>
      </header>
    )
  }
}