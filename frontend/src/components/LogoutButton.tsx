import { FunctionComponent } from 'preact'

interface LogoutButtonProps {
  onLogout: () => void
}

export const LogoutButton: FunctionComponent<LogoutButtonProps> = ({ onLogout }) => (
  <button
    onClick={onLogout}
    class="w-full mt-6 px-4 py-2.5 bg-[var(--color-bg-primary)]
           text-[var(--color-text-secondary)] hover:text-red-500
           rounded-lg hover:bg-[var(--color-bg-tertiary)]
           transition-all flex items-center justify-center gap-2
           font-medium text-sm border border-[var(--color-border)]"
  >
    <span class="text-base">🚪</span>
    Sign Out
  </button>
)