interface BurgerButtonProps {
  isOpen: boolean;
  onClick: () => void;
}

export function BurgerButton({ isOpen, onClick }: BurgerButtonProps) {
  return (
    <button
      onClick={onClick}
      class="lg:hidden fixed top-4 left-4 z-50 p-2 rounded-lg bg-[var(--color-bg-secondary)] hover:bg-[var(--color-bg-tertiary)] transition-colors"
      aria-label={isOpen ? "Close menu" : "Open menu"}
    >
      <div class="w-6 h-5 relative flex flex-col justify-between">
        <span
          class={`w-full h-0.5 bg-[var(--color-text-primary)] transition-transform origin-left
            ${isOpen ? "rotate-45 translate-x-0.5" : ""}`}
        />
        <span
          class={`w-full h-0.5 bg-[var(--color-text-primary)] transition-opacity
            ${isOpen ? "opacity-0" : ""}`}
        />
        <span
          class={`w-full h-0.5 bg-[var(--color-text-primary)] transition-transform origin-left
            ${isOpen ? "-rotate-45 translate-x-0.5" : ""}`}
        />
      </div>
    </button>
  );
} 