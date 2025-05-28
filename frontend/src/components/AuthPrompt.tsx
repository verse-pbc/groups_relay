import { FunctionComponent } from 'preact'
import { useState } from 'preact/hooks'
import { nip19 } from 'nostr-tools'

interface AuthPromptProps {
  onSubmit: (key: string) => void
}

export const AuthPrompt: FunctionComponent<AuthPromptProps> = ({ onSubmit }) => {
  const [key, setKey] = useState('')
  const [error, setError] = useState('')
  const [isConnecting, setIsConnecting] = useState(false)

  // Convert nsec to hex if needed, return hex private key
  const processPrivateKey = (key: string): string | null => {
    const cleanKey = key.trim();
    
    if (cleanKey.startsWith('nsec')) {
      try {
        const decoded = nip19.decode(cleanKey);
        if (decoded.type === 'nsec') {
          // Convert Uint8Array to hex string
          return Array.from(decoded.data as Uint8Array)
            .map(b => b.toString(16).padStart(2, '0'))
            .join('');
        }
        return null;
      } catch (e) {
        return null;
      }
    }

    // Check if it's a valid hex string
    const hexRegex = /^[0-9a-fA-F]+$/;
    if (!hexRegex.test(cleanKey)) {
      return null;
    }

    // Check length (32 bytes = 64 hex characters)
    if (cleanKey.length !== 64) {
      return null;
    }

    return cleanKey;
  }

  const handleSubmit = (e: Event) => {
    e.preventDefault()
    if (!key) {
      setError('A private key is required')
      return
    }

    // Process and validate the key
    const hexKey = processPrivateKey(key);
    if (!hexKey) {
      setError('Invalid private key format. Please enter a valid nsec key or 64-character hex string.')
      return
    }

    // Always pass the hex version to maintain backward compatibility
    onSubmit(hexKey)
  }

  const handleConnectExtension = async () => {
    setIsConnecting(true)
    setError('')

    try {
      if (!window.nostr) {
        setError('No Nostr extension found. Please install one first.')
        return
      }

      const pubkey = await window.nostr.getPublicKey()
      if (pubkey) {
        onSubmit(pubkey)
      }
    } catch (e) {
      console.error('Failed to connect to extension:', e)
      setError('Failed to connect to extension. Please try again.')
    } finally {
      setIsConnecting(false)
    }
  }

  return (
    <div class="min-h-screen flex items-center justify-center bg-[var(--color-bg-primary)]">
      <div class="w-full max-w-md">
        {/* Logo/Brand Section */}
        <div class="text-center mb-8">
          <div class="inline-flex items-center justify-center w-16 h-16 rounded-full bg-accent/10 mb-4">
            <svg xmlns="http://www.w3.org/2000/svg" class="h-8 w-8 text-accent" viewBox="0 0 20 20" fill="currentColor">
              <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM9.555 7.168A1 1 0 008 8v4a1 1 0 001.555.832l3-2a1 1 0 000-1.664l-3-2z" clip-rule="evenodd" />
            </svg>
          </div>
          <h1 class="text-2xl font-bold text-[var(--color-text-primary)]">Nostr Groups</h1>
          <p class="mt-2 text-[var(--color-text-secondary)]">Connect to start collaborating</p>
        </div>

        {/* Main Card */}
        <div class="bg-[var(--color-bg-secondary)] rounded-xl shadow-lg border border-[var(--color-border)] p-6">
          <div class="space-y-6">
            {/* Extension Button */}
            <div>
              <button
                onClick={handleConnectExtension}
                disabled={isConnecting}
                class="w-full px-4 py-3 bg-accent text-white rounded-lg hover:bg-accent/90
                       disabled:opacity-50 disabled:cursor-not-allowed transition-all
                       flex items-center justify-center gap-2 shadow-sm hover:shadow-md
                       transform hover:-translate-y-0.5 active:translate-y-0"
              >
                {isConnecting ? (
                  <>
                    <span class="animate-spin">⚡</span>
                    Connecting...
                  </>
                ) : (
                  <>
                    <svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5" viewBox="0 0 20 20" fill="currentColor">
                      <path fill-rule="evenodd" d="M11.3 1.046A1 1 0 0112 2v5h4a1 1 0 01.82 1.573l-7 10A1 1 0 018 18v-5H4a1 1 0 01-.82-1.573l7-10a1 1 0 011.12-.38z" clip-rule="evenodd" />
                    </svg>
                    Connect with Extension
                  </>
                )}
              </button>
            </div>

            {/* Divider */}
            <div class="relative">
              <div class="absolute inset-0 flex items-center">
                <div class="w-full border-t border-[var(--color-border)]"></div>
              </div>
              <div class="relative flex justify-center text-xs uppercase">
                <span class="px-4 bg-[var(--color-bg-secondary)] text-[var(--color-text-tertiary)]">
                  Or continue with
                </span>
              </div>
            </div>

            {/* Private Key Form */}
            <form onSubmit={handleSubmit} class="space-y-4">
              <div class="space-y-2">
                <label class="block text-sm font-medium text-[var(--color-text-secondary)]">
                  Private Key
                </label>
                <div class="relative">
                  <input
                    type="password"
                    value={key}
                    onChange={(e) => setKey((e.target as HTMLInputElement).value)}
                    placeholder="nsec..."
                    autocomplete="off"
                    class="w-full p-3 pl-10 border rounded-lg bg-[var(--color-bg-primary)]
                           text-[var(--color-text-primary)] placeholder-[var(--color-text-tertiary)]/40
                           focus:outline-none focus:ring-2 focus:ring-accent/20 focus:border-accent
                           transition-all"
                  />
                  <div class="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none">
                    <svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 text-[var(--color-text-tertiary)]" viewBox="0 0 20 20" fill="currentColor">
                      <path fill-rule="evenodd" d="M5 9V7a5 5 0 0110 0v2a2 2 0 012 2v5a2 2 0 01-2 2H5a2 2 0 01-2-2v-5a2 2 0 012-2zm8-2v2H7V7a3 3 0 016 0z" clip-rule="evenodd" />
                    </svg>
                  </div>
                </div>
              </div>

              <button
                type="submit"
                class="w-full px-4 py-3 border-2 border-accent text-accent rounded-lg
                       hover:bg-accent hover:text-white transition-all
                       transform hover:-translate-y-0.5 active:translate-y-0
                       focus:outline-none focus:ring-2 focus:ring-accent/20"
              >
                Connect with nsec
              </button>
            </form>
          </div>

          {error && (
            <div class="mt-6 p-4 bg-red-50 border border-red-100 rounded-lg text-red-600 text-sm flex items-start gap-2">
              <svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5 flex-shrink-0 mt-0.5" viewBox="0 0 20 20" fill="currentColor">
                <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd" />
              </svg>
              <span>{error}</span>
            </div>
          )}
        </div>

        {/* Footer */}
        <div class="mt-8 text-center text-sm text-[var(--color-text-tertiary)]">
          <p>Need a Nostr extension? Try{' '}
            <a href="https://getalby.com" target="_blank" rel="noopener noreferrer"
               class="text-accent hover:text-accent/90 transition-colors">
              Alby
            </a>
            {' '}or{' '}
            <a href="https://github.com/fiatjaf/nos2x" target="_blank" rel="noopener noreferrer"
               class="text-accent hover:text-accent/90 transition-colors">
              nos2x
            </a>
          </p>
        </div>
      </div>
    </div>
  )
}