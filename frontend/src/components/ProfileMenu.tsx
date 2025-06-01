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
    showWalletModal: false
  };

  componentDidMount() {
    console.log('🔔 [PROFILE] ProfileMenu mounting...');
    document.addEventListener('mousedown', this.handleClickOutside);
    document.addEventListener('keydown', this.handleKeyDown);

    // Subscribe to balance updates IMMEDIATELY on mount
    console.log('🔔 [PROFILE] Subscribing to balance updates IMMEDIATELY');
    this.unsubscribeBalance = this.props.client.onBalanceUpdate((balance) => {
      console.log('🔔 [PROFILE] Balance update received:', balance);
      this.setState({ cashuBalance: balance });
    });
    console.log('🔔 [PROFILE] Subscription complete');

    // Get user info first to get pubkey for cached balance
    this.loadUserInfo();

  }

  loadUserInfo = async () => {
    const user = await this.props.client.ndkInstance.signer?.user();
    console.log('🔔 [PROFILE] User pubkey:', user?.pubkey ? 'found' : 'not found');
    if (user?.pubkey) {
      this.setState({ userPubkey: user.pubkey });

      // Try to get cached balance immediately with pubkey
      const cachedBalance = this.props.client.getCachedBalanceForUser(user.pubkey);
      if (cachedBalance > 0) {
        console.log('🔔 [PROFILE] Loaded cached balance:', cachedBalance);
        this.setState({ cashuBalance: cachedBalance });
      }

      // Run admin check and wallet initialization in parallel
      const isAdmin = await this.props.client.checkIsRelayAdmin().catch(error => {
        console.error('Failed to check relay admin status:', error);
        return false;
      });

      if (isAdmin) {
        this.setState({ isRelayAdmin: true });
      }

      // Initialize wallet if not already done
      if (!this.props.client.isWalletInitialized()) {
        console.log('🔔 [PROFILE] Wallet not initialized, initializing...');
        try {
          await this.props.client.initializeWallet();
          console.log('🔔 [PROFILE] Wallet initialized successfully');
        } catch (err) {
          console.warn('🔔 [PROFILE] Failed to initialize wallet:', err);
        }
      }

      // Fetch fresh balance after user is loaded
      try {
        const balance = await this.props.client.getCashuBalance();
        console.log('🔔 [PROFILE] Fresh balance fetched:', balance);
        this.setState({ cashuBalance: balance });
      } catch (err) {
        console.warn('🔔 [PROFILE] Failed to fetch fresh balance:', err);
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
          
          {/* Show total balance */}
          <div class="flex items-center gap-2 text-sm">
            <div class="text-[#f7931a] font-semibold flex items-center gap-1">
              {(() => {
                const totalBalance = this.state.cashuBalance;
                return totalBalance > 0 ? (
                  <>
                    <span class="text-sm">₿</span>
                    {totalBalance.toLocaleString()}
                    <span class="text-xs font-normal">sats</span>
                  </>
                ) : null;
              })()}
            </div>
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
              
              <div class="flex items-center justify-between">
                <span class="text-xs text-[var(--color-text-secondary)]">Balance:</span>
                <span class="text-sm font-semibold text-[#f7931a] flex items-center gap-1">
                  <span>₿</span>
                  {this.state.cashuBalance.toLocaleString()}
                  <span class="text-xs font-normal">sats</span>
                </span>
              </div>
              
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
              onClose={() => {
                this.setState({ showWalletModal: false });
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