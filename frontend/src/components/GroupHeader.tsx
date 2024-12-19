import { FunctionComponent } from 'preact'
import type { Group } from '../types'

interface GroupHeaderProps {
  group: Group
  isEditingName: boolean
  newName: string
  onNameEdit: () => void
  onNameSave: () => void
  onNameChange: (name: string) => void
}

export const GroupHeader: FunctionComponent<GroupHeaderProps> = ({
  group,
  isEditingName,
  newName,
  onNameEdit,
  onNameSave,
  onNameChange,
}) => {
  return (
    <div class="p-4 border-b border-[var(--color-border)] bg-[var(--color-bg-secondary)]">
      <div class="flex items-start gap-3">
        {group.picture ? (
          <img
            src={group.picture}
            alt={group.name}
            class="w-12 h-12 rounded-lg object-cover bg-[var(--color-bg-primary)]"
          />
        ) : (
          <div class="w-12 h-12 rounded-lg bg-accent/10 flex items-center justify-center">
            <span class="text-lg font-medium text-accent">
              {(group.name || '?').charAt(0).toUpperCase()}
            </span>
          </div>
        )}

        <div class="flex-1 min-w-0">
          <div class="flex items-center justify-between gap-2">
            {isEditingName ? (
              <div class="w-full space-y-2">
                <input
                  type="text"
                  value={newName}
                  onChange={(e) => onNameChange((e.target as HTMLInputElement).value)}
                  class="w-full px-3 py-1.5 bg-[var(--color-bg-primary)] border border-[var(--color-border)]
                         rounded-lg text-base text-[var(--color-text-primary)]
                         placeholder-[var(--color-text-tertiary)]
                         focus:outline-none focus:ring-1 focus:ring-accent"
                  placeholder="Enter group name"
                  autoFocus
                />
                <div class="flex justify-end gap-2">
                  <button
                    onClick={() => onNameChange(group.name || '')}
                    class="px-2 py-1 text-xs text-[var(--color-text-secondary)]
                           hover:text-[var(--color-text-primary)] transition-colors"
                  >
                    Cancel
                  </button>
                  <button
                    onClick={onNameSave}
                    class="px-2 py-1 bg-accent text-white text-xs rounded
                           hover:bg-accent/90 transition-colors"
                  >
                    Save
                  </button>
                </div>
              </div>
            ) : (
              <>
                <h2 class="text-lg font-medium text-[var(--color-text-primary)] truncate">
                  {group.name || 'Unnamed Group'}
                </h2>
                <button
                  onClick={onNameEdit}
                  class="shrink-0 text-xs text-accent hover:text-accent/90 transition-colors"
                >
                  Edit
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}