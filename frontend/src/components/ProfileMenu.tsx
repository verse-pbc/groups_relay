import { Component } from 'preact';
import { NostrClient } from '../api/nostr_client';
import { UserDisplay } from './UserDisplay';
import { WalletDisplay } from './WalletDisplay';
import type { Proof } from '@cashu/cashu-ts';

interface ProfileMenuProps {
  client: NostrClient;
  onLogout: () => void;
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void;
  cashuProofs?: Proof[];
  mints?: string[];
  onNutzapSent?: () => void;
  onOpenWallet?: () => void;
}

interface ProfileMenuState {
  showMenu: boolean;
  userPubkey: string | null;
  isRelayAdmin: boolean;
  cashuBalance: number;
  showWalletModal: boolean;
  mintCount: number;
  walletLoading: boolean;
}

export class ProfileMenu extends Component<ProfileMenuProps, ProfileMenuState> {
  private menuRef = null as HTMLDivElement | null;
  private buttonRef = null as HTMLButtonElement | null;
  private unsubscribeBalance: (() => void) | null = null;

  state = {
    showMenu: false,
    userPubkey: null,
    isRelayAdmin: false,
    cashuBalance: 0,
    showWalletModal: false,
    mintCount: 0,
    walletLoading: true
  };

  componentDidMount() {
    document.addEventListener('mousedown', this.handleClickOutside);
    document.addEventListener('keydown', this.handleKeyDown);

    // Subscribe to balance updates IMMEDIATELY on mount
    this.unsubscribeBalance = this.props.client.onBalanceUpdate(async (balance) => {
      this.setState({ cashuBalance: balance });
      
      // Also refresh mint count when balance updates
      try {
        const mints = await this.props.client.getCashuMints();
        this.setState({ mintCount: mints.length });
      } catch (err) {
        // Failed to refresh mint count on balance update
      }
    });

    // Get user info first to get pubkey for cached balance
    this.loadUserInfo();

  }

  loadUserInfo = async () => {
    const user = await this.props.client.ndkInstance.signer?.user();
    if (user?.pubkey) {
      this.setState({ userPubkey: user.pubkey });

      // Try to get cached balance immediately with pubkey
      const cachedBalance = this.props.client.getCachedBalanceForUser(user.pubkey);
      if (cachedBalance > 0) {
        this.setState({ cashuBalance: cachedBalance });
      }

      // Run admin check and wallet initialization in parallel
      const isAdmin = await this.props.client.checkIsRelayAdmin().catch(() => {
        return false;
      });

      if (isAdmin) {
        this.setState({ isRelayAdmin: true });
      }

      // Initialize wallet if not already done
      if (!this.props.client.isWalletInitialized()) {
        try {
          await this.props.client.initializeWallet();
          
          // Get mint count after initialization
          const mints = await this.props.client.getCashuMints();
          this.setState({ mintCount: mints.length, walletLoading: false });
        } catch (err) {
          this.setState({ walletLoading: false });
        }
      } else {
        // Wallet already initialized, get mint count
        const mints = await this.props.client.getCashuMints();
        this.setState({ mintCount: mints.length, walletLoading: false });
      }

      // Fetch fresh balance and mint count after user is loaded
      try {
        const [balance, mints] = await Promise.all([
          this.props.client.getCashuBalance(),
          this.props.client.getCashuMints()
        ]);
        this.setState({ cashuBalance: balance, mintCount: mints.length });
      } catch (err) {
        // Failed to fetch fresh balance
      }
    }
  }


  componentWillUnmount() {
    document.removeEventListener('mousedown', this.handleClickOutside);
    document.removeEventListener('keydown', this.handleKeyDown);
    
    // Unsubscribe from balance updates
    if (this.unsubscribeBalance) {
      this.unsubscribeBalance();
    }
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
    const { showMenu, userPubkey, isRelayAdmin, showWalletModal } = this.state;

    if (!userPubkey) return null;

    return (
      <>
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
            hideNutzap={!window.location.search.includes('selfnutzap')}
          />
          
          {/* Show total balance or CTA */}
          <div class="flex items-center gap-2 text-sm">
            {this.state.walletLoading ? (
              <div class="text-[var(--color-text-tertiary)] text-sm">
                Loading...
              </div>
            ) : this.state.mintCount === 0 ? (
              <button
                onClick={() => {
                  this.setState({ showMenu: false, showWalletModal: true });
                }}
                class="text-purple-400 hover:text-purple-300 font-semibold text-sm transition-colors"
              >
                Add Mints →
              </button>
            ) : (
              <div class="text-[#f7931a] font-semibold flex items-center gap-1">
                <span class="text-sm">₿</span>
                {this.state.cashuBalance.toLocaleString()}
                <span class="text-xs font-normal">sats</span>
              </div>
            )}
            <svg
              class={`w-4 h-4 text-[var(--color-text-tertiary)] transition-transform duration-200 ${showMenu ? 'rotate-180' : ''}`}
              viewBox="0 0 24 24"
              fill="none"
              xmlns="http://www.w3.org/2000/svg"
              aria-hidden="true"
            >
              <path d="M6 9l6 6 6-6" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </div>
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

            {/* Wallet Section */}
            <div class="px-4 py-3">
              <div class="text-xs font-medium text-[var(--color-text-tertiary)] mb-2">
                Wallet
              </div>
              
              {this.state.walletLoading ? (
                <div class="text-[var(--color-text-tertiary)] text-sm py-2">
                  Loading wallet...
                </div>
              ) : this.state.mintCount === 0 ? (
                <div class="bg-yellow-900/20 border border-yellow-700/30 rounded-lg p-3 mb-2">
                  <p class="text-xs text-yellow-400">
                    ⚠️ No mints configured. Add a mint to start using your wallet.
                  </p>
                </div>
              ) : (
                <div class="flex items-center justify-between">
                  <span class="text-xs text-[var(--color-text-secondary)]">Balance:</span>
                  <span class="text-sm font-semibold text-[#f7931a] flex items-center gap-1">
                    <span>₿</span>
                    {this.state.cashuBalance.toLocaleString()}
                    <span class="text-xs font-normal">sats</span>
                  </span>
                </div>
              )}
              
              <div class="mt-3 space-y-2">
                <button
                  onClick={() => {
                    this.setState({ showMenu: false, showWalletModal: true });
                  }}
                  class="w-full px-3 py-2 bg-purple-600 hover:bg-purple-700 rounded-md text-xs font-medium transition-colors"
                >
                  Open Wallet
                </button>
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
      
      {/* Wallet Modal */}
      {showWalletModal && (
        <div 
          class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50 p-4"
          onClick={(e) => {
            // Close modal if clicking backdrop
            if (e.target === e.currentTarget) {
              this.setState({ showWalletModal: false });
            }
          }}
        >
          <div class="w-full max-w-md">
            <WalletDisplay 
              client={client} 
              onClose={async () => {
                this.setState({ showWalletModal: false });
                // Refresh mint count when closing wallet modal
                try {
                  const mints = await this.props.client.getCashuMints();
                  this.setState({ mintCount: mints.length });
                } catch (err) {
                  // Failed to refresh mint count
                }
              }}
              isModal={true}
              initialCashuBalance={this.state.cashuBalance}
              walletBalance={this.state.cashuBalance}
              isWalletInitialized={this.props.client.isWalletInitialized()}
            />
          </div>
        </div>
      )}
      </>
    );
  }
}