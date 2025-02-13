import { Group } from "../types";

interface GroupSidebarProps {
  groups: Group[];
  selectedGroupId?: string;
  onSelectGroup: (group: Group) => void;
}

export function GroupSidebar({ groups, selectedGroupId, onSelectGroup }: GroupSidebarProps) {
  return (
    <div class="mt-6">
      <h3 class="text-sm font-medium text-[var(--color-text-secondary)] mb-3">Your Groups</h3>
      <div class="space-y-1">
        {groups.map((group) => (
          <button
            key={group.id}
            onClick={() => onSelectGroup(group)}
            class={`w-full text-left px-3 py-2 rounded-lg text-sm transition-colors
              ${
                group.id === selectedGroupId
                  ? "bg-accent text-white"
                  : "hover:bg-[var(--color-bg-secondary)] text-[var(--color-text-primary)]"
              }`}
          >
            <div class="flex items-center gap-2">
              {group.picture ? (
                <img
                  src={group.picture}
                  alt=""
                  class="w-6 h-6 rounded-full object-cover border border-[var(--color-border)]"
                  onError={(e) => {
                    (e.target as HTMLImageElement).style.display = 'none';
                    const parent = e.currentTarget.parentElement;
                    if (parent) {
                      const fallback = document.createElement('div');
                      fallback.className = 'w-6 h-6 rounded-full bg-[var(--color-bg-primary)] border border-[var(--color-border)] flex items-center justify-center text-sm font-medium';
                      fallback.textContent = group.name?.charAt(0).toUpperCase() || 'G';
                      parent.insertBefore(fallback, e.currentTarget);
                    }
                  }}
                />
              ) : (
                <div class="w-6 h-6 rounded-full bg-[var(--color-bg-primary)] border border-[var(--color-border)] flex items-center justify-center text-sm font-medium">
                  {group.name?.charAt(0).toUpperCase() || 'G'}
                </div>
              )}
              <span class="truncate">{group.name || group.id}</span>
            </div>
          </button>
        ))}
        {groups.length === 0 && (
          <p class="text-sm text-[var(--color-text-tertiary)] px-3 py-2">
            No groups yet
          </p>
        )}
      </div>
    </div>
  );
} 