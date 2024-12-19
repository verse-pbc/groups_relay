import { FunctionComponent } from 'preact'
import type { Group } from '../types'

interface GroupInfoProps {
  group: Group
  isEditingAbout: boolean
  newAbout: string
  onAboutEdit: () => void
  onAboutSave: () => void
  onAboutChange: (about: string) => void
  onMetadataChange: (changes: Partial<Group>) => void
}

export const GroupInfo: FunctionComponent<GroupInfoProps> = ({
  group,
  isEditingAbout,
  newAbout,
  onAboutEdit,
  onAboutSave,
  onAboutChange,
  onMetadataChange,
}) => {
  return (
    <div class="space-y-4">
      {/* About Section */}
      <div class="space-y-2">
        <div class="flex items-center justify-between">
          <label class="text-xs font-medium text-[var(--color-text-secondary)]">About</label>
          {!isEditingAbout && (
            <button
              onClick={onAboutEdit}
              class="text-xs text-accent hover:text-accent/90 transition-colors"
            >
              Edit
            </button>
          )}
        </div>

        {isEditingAbout ? (
          <div class="space-y-2">
            <textarea
              value={newAbout}
              onChange={(e) => onAboutChange((e.target as HTMLTextAreaElement).value)}
              rows={3}
              class="w-full px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)]
                     rounded-lg text-sm text-[var(--color-text-primary)]
                     placeholder-[var(--color-text-tertiary)]
                     focus:outline-none focus:ring-1 focus:ring-accent"
              placeholder="Enter group description"
              autoFocus
            />
            <div class="flex justify-end gap-2">
              <button
                onClick={() => onAboutChange(group.about || '')}
                class="px-2 py-1 text-xs text-[var(--color-text-secondary)]
                       hover:text-[var(--color-text-primary)] transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={onAboutSave}
                class="px-2 py-1 bg-accent text-white text-xs rounded
                       hover:bg-accent/90 transition-colors"
              >
                Save
              </button>
            </div>
          </div>
        ) : (
          <p class="text-sm text-[var(--color-text-primary)] leading-relaxed">
            {group.about || 'No description provided'}
          </p>
        )}
      </div>

      {/* Settings Section */}
      <div class="space-y-2">
        <label class="text-xs font-medium text-[var(--color-text-secondary)]">Settings</label>
        <div class="space-y-3 p-3 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)]">
          {/* Private Toggle */}
          <div class="flex items-center justify-between">
            <div>
              <div class="text-sm font-medium text-[var(--color-text-primary)]">Private Group</div>
              <div class="text-xs text-[var(--color-text-tertiary)]">Only members can see group content</div>
            </div>
            <button
              onClick={() => onMetadataChange({ private: !group.private })}
              class={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors
                     focus:outline-none focus:ring-2 focus:ring-accent focus:ring-offset-2
                     focus:ring-offset-[var(--color-bg-primary)]
                     ${group.private
                       ? 'bg-accent'
                       : 'bg-[var(--color-bg-tertiary)] dark:bg-[var(--color-text-tertiary)]'
                     }`}
              role="switch"
              aria-checked={group.private}
            >
              <span
                class={`inline-block h-5 w-5 transform rounded-full bg-white shadow-sm
                        transition-transform duration-200 ease-in-out
                        ${group.private ? 'translate-x-6' : 'translate-x-0.5'}`}
              />
            </button>
          </div>

          {/* Closed Toggle */}
          <div class="flex items-center justify-between">
            <div>
              <div class="text-sm font-medium text-[var(--color-text-primary)]">Closed Group</div>
              <div class="text-xs text-[var(--color-text-tertiary)]">Only admins can invite new members</div>
            </div>
            <button
              onClick={() => onMetadataChange({ closed: !group.closed })}
              class={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors
                     focus:outline-none focus:ring-2 focus:ring-accent focus:ring-offset-2
                     focus:ring-offset-[var(--color-bg-primary)]
                     ${group.closed
                       ? 'bg-accent'
                       : 'bg-[var(--color-bg-tertiary)] dark:bg-[var(--color-text-tertiary)]'
                     }`}
              role="switch"
              aria-checked={group.closed}
            >
              <span
                class={`inline-block h-5 w-5 transform rounded-full bg-white shadow-sm
                        transition-transform duration-200 ease-in-out
                        ${group.closed ? 'translate-x-6' : 'translate-x-0.5'}`}
              />
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}