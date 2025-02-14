import { Component } from 'preact';
import { NostrClient } from '../api/nostr_client';
import { UserDisplay } from './UserDisplay';

interface ProfileMenuProps {
  client: NostrClient;
  onLogout: () => void;
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void;
}

interface ProfileMenuState {
  showMenu: boolean;
  userPubkey: string | null;
  isRelayAdmin: boolean;
}

export class ProfileMenu extends Component<ProfileMenuProps, ProfileMenuState> {
  private menuRef = null as HTMLDivElement | null;
  private buttonRef = null as HTMLButtonElement | null;

  state = {
    showMenu: false,
    userPubkey: null,
    isRelayAdmin: false
  };

  async componentDidMount() {
    document.addEventListener('mousedown', this.handleClickOutside);
    document.addEventListener('keydown', this.handleKeyDown);

    const user = await this.props.client.ndkInstance.signer?.user();
    if (user?.pubkey) {
      this.setState({ userPubkey: user.pubkey });

      // Check if user is relay admin
      try {
        const relayUrl = this.props.client.config.relayUrl;
        const httpUrl = relayUrl.replace(/^ws/, 'http');

        const response = await fetch(httpUrl, {
          headers: {
            'Accept': 'application/nostr+json'
          }
        });
        if (response.ok) {
          const relayInfo = await response.json();
          const relayPubkey = relayInfo.pubkey;
          if (relayPubkey === user.pubkey) {
            this.setState({ isRelayAdmin: true });
          }
        }
      } catch (error) {
        console.error('Failed to check relay admin status:', error);
      }
    }
  }

  componentWillUnmount() {
    document.removeEventListener('mousedown', this.handleClickOutside);
    document.removeEventListener('keydown', this.handleKeyDown);
  }

  handleClickOutside = (event: MouseEvent) => {
    if (this.menuRef && !this.menuRef.contains(event.target as Node)) {
      this.setState({ showMenu: false });
    }
  };

  handleKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape') {
      this.setState({ showMenu: false });
    } else if (event.key === 'Tab') {
      // If tabbing and menu is open, trap focus within menu
      if (this.state.showMenu) {
        const menu = this.menuRef;
        const button = this.buttonRef;
        if (!menu || !button) return;

        const focusableElements = menu.querySelectorAll(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
        );
        const firstElement = focusableElements[0] as HTMLElement;
        const lastElement = focusableElements[focusableElements.length - 1] as HTMLElement;

        if (event.shiftKey) {
          // If shift+tab and on first element, move to last
          if (document.activeElement === firstElement) {
            event.preventDefault();
            lastElement.focus();
          }
        } else {
          // If tab and on last element, move to first
          if (document.activeElement === lastElement) {
            event.preventDefault();
            firstElement.focus();
          }
        }
      }
    }
  };

  toggleMenu = () => {
    this.setState(state => ({ showMenu: !state.showMenu }));
  };

  render() {
    const { client, onLogout } = this.props;
    const { showMenu, userPubkey, isRelayAdmin } = this.state;

    if (!userPubkey) return null;

    return (
      <div class="relative" ref={el => this.menuRef = el}>
        <button
          ref={el => this.buttonRef = el}
          type="button"
          onClick={this.toggleMenu}
          class="flex items-center gap-2 p-2 rounded-lg hover:bg-[var(--color-bg-primary)] transition-colors"
          aria-expanded={showMenu}
          aria-haspopup="true"
          aria-label="Profile menu"
        >
          <UserDisplay
            pubkey={client.pubkeyToNpub(userPubkey)}
            client={client}
            showCopy={false}
            size="md"
            isRelayAdmin={isRelayAdmin}
          />
          <svg
            class={`w-4 h-4 text-[var(--color-text-tertiary)] transition-transform duration-200 ${showMenu ? 'rotate-180' : ''}`}
            viewBox="0 0 24 24"
            fill="none"
            xmlns="http://www.w3.org/2000/svg"
            aria-hidden="true"
          >
            <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
        </button>

        {showMenu && (
          <div
            class="absolute right-0 mt-2 w-72 bg-[var(--color-bg-primary)] rounded-lg border border-[var(--color-border)] shadow-lg overflow-hidden divide-y divide-[var(--color-border)]"
            role="menu"
          >
            {/* Profile Info */}
            <div class="px-4 py-3">
              <div class="text-xs font-medium text-[var(--color-text-tertiary)] mb-1">
                Signed in as
              </div>
              <div class="text-sm font-medium text-[var(--color-text-primary)] truncate">
                {client.pubkeyToNpub(userPubkey)}
              </div>
            </div>

            {/* Actions */}
            <div>
              <button
                type="button"
                onClick={onLogout}
                class="w-full px-4 py-3 text-left text-sm text-red-400 hover:bg-[var(--color-bg-secondary)] transition-colors flex items-center gap-2"
                role="menuitem"
              >
                <svg class="w-4 h-4" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
                  <path d="M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                  <path d="M16 17l5-5-5-5M21 12H9" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                </svg>
                Sign out
              </button>
            </div>
          </div>
        )}
      </div>
    );
  }
}