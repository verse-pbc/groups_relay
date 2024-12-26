import { Component } from 'preact'
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

export class GroupInfo extends Component<GroupInfoProps> {
  render() {
    const { group, isEditingAbout, newAbout, onAboutEdit, onAboutSave, onAboutChange } = this.props

    return (
      <div class="space-y-6">
        {/* Settings */}
        <div>
          <div class="border-b border-[var(--color-border)] pb-3">
            <h3 class="text-base font-semibold leading-6 text-[var(--color-text-primary)] flex items-center gap-2">
              <span class="text-[var(--color-text-secondary)]">⚙️</span>
              Settings
            </h3>
          </div>

          <div class="mt-4 space-y-4 bg-[var(--color-bg-primary)] rounded-lg p-4">
            {/* Private Group Toggle */}
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-3">
                <span class="text-lg">🔒</span>
                <div>
                  <div class="font-medium">Private Group</div>
                  <div class="text-sm text-[var(--color-text-tertiary)]">Only members can see group content</div>
                </div>
              </div>
              <button
                type="button"
                onClick={() => this.props.onMetadataChange({ private: !group.private })}
                class={`${
                  group.private ? 'bg-[var(--color-accent)]' : 'bg-[#2A2B2E]'
                } relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-[var(--color-accent)] focus:ring-offset-2`}
                role="switch"
                aria-checked={group.private}
              >
                <span class="sr-only">Private group setting</span>
                <span
                  aria-hidden="true"
                  class={`${
                    group.private ? 'translate-x-5' : 'translate-x-0'
                  } pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out`}
                />
              </button>
            </div>

            {/* Closed Group Toggle */}
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-3">
                <span class="text-lg">👥</span>
                <div>
                  <div class="font-medium">Closed Group</div>
                  <div class="text-sm text-[var(--color-text-tertiary)]">Only admins can invite new members</div>
                </div>
              </div>
              <button
                type="button"
                onClick={() => this.props.onMetadataChange({ closed: !group.closed })}
                class={`${
                  group.closed ? 'bg-[var(--color-accent)]' : 'bg-[#2A2B2E]'
                } relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-[var(--color-accent)] focus:ring-offset-2`}
                role="switch"
                aria-checked={group.closed}
              >
                <span class="sr-only">Closed group setting</span>
                <span
                  aria-hidden="true"
                  class={`${
                    group.closed ? 'translate-x-5' : 'translate-x-0'
                  } pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out`}
                />
              </button>
            </div>
          </div>
        </div>

        {/* About */}
        <div>
          <div class="border-b border-[var(--color-border)] pb-3 flex items-center justify-between">
            <h3 class="text-base font-semibold leading-6 text-[var(--color-text-primary)] flex items-center gap-2">
              <span class="text-[var(--color-text-secondary)]">ℹ️</span>
              About
            </h3>
            {!isEditingAbout && (
              <button
                onClick={onAboutEdit}
                class="text-sm font-medium text-[var(--color-accent)] hover:text-[var(--color-accent)]/90 transition-colors"
              >
                Edit
              </button>
            )}
          </div>

          <div class="mt-4">
            {isEditingAbout ? (
              <div class="space-y-3">
                <textarea
                  value={newAbout}
                  onInput={(e) => onAboutChange((e.target as HTMLTextAreaElement).value)}
                  class="w-full h-24 px-3 py-2 bg-[var(--color-bg-primary)] border border-[var(--color-border)]
                         rounded-lg text-sm text-[var(--color-text-primary)]
                         placeholder-[var(--color-text-tertiary)]
                         focus:outline-none focus:ring-1 focus:ring-[var(--color-accent)]
                         hover:border-[var(--color-border-hover)] transition-colors resize-none"
                  placeholder="Enter group description"
                />
                <div class="flex justify-end gap-2">
                  <button
                    onClick={() => onAboutChange(group.about || '')}
                    class="px-3 py-1.5 text-sm font-medium text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
                  >
                    Cancel
                  </button>
                  <button
                    onClick={onAboutSave}
                    class="px-3 py-1.5 text-sm font-medium text-[var(--color-accent)] hover:text-[var(--color-accent)]/90 transition-colors"
                  >
                    Save
                  </button>
                </div>
              </div>
            ) : (
              <p class="text-sm text-[var(--color-text-secondary)]">
                {group.about || 'No description provided'}
              </p>
            )}
          </div>
        </div>
      </div>
    )
  }
}