interface BurgerButtonProps {
  isOpen: boolean;
  onClick: () => void;
}

export function BurgerButton({ isOpen, onClick }: BurgerButtonProps) {
  return (
    <button
      onClick={onClick}
      class="w-8 h-8 flex-shrink-0 flex items-center justify-center focus:outline-none"
      aria-label={isOpen ? "Close menu" : "Open menu"}
    >
      <div class="relative w-5 h-4">
        <span
          class={`absolute h-0.5 w-full bg-current transform transition-all duration-200 ease-in-out ${
            isOpen ? "rotate-45 translate-y-1.5" : "translate-y-0"
          }`}
        />
        <span
          class={`absolute h-0.5 w-full bg-current transform transition-all duration-200 ease-in-out ${
            isOpen ? "opacity-0" : "translate-y-1.5"
          }`}
        />
        <span
          class={`absolute h-0.5 w-full bg-current transform transition-all duration-200 ease-in-out ${
            isOpen ? "-rotate-45 translate-y-1.5" : "translate-y-3"
          }`}
        />
      </div>
    </button>
  );
}