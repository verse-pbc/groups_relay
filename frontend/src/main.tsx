import 'preact/debug'
import 'preact/devtools'
import { render } from 'preact'
import { useState, useEffect } from 'preact/hooks'
import { NostrClient } from './api/nostr_client.ts'
import { App } from './components/App.tsx'
import { LoadingState } from './components/LoadingState.tsx'
import { ErrorState } from './components/ErrorState.tsx'
import { AuthPrompt } from './components/AuthPrompt.tsx'
import './style.css'

// Get WebSocket URL from environment variable or use current host
const getWebSocketUrl = () => {
  // Check if we have an environment variable for the WebSocket URL
  if (import.meta.env.VITE_WEBSOCKET_URL) {
    return import.meta.env.VITE_WEBSOCKET_URL;
  }

  // Otherwise, use the current host
  return `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}`;
};

const wsUrl = getWebSocketUrl();
console.log('Using WebSocket URL:', wsUrl);

interface InitializationProps {
  onComplete: (client: NostrClient) => void
}

const Initialization = ({ onComplete }: InitializationProps) => {
  const [error, setError] = useState<Error | null>(null)
  const [status, setStatus] = useState<'idle' | 'connecting'>('idle')

  useEffect(() => {
    // Check if we have a stored key
    const storedKey = localStorage.getItem('nostr_key')
    if (storedKey) {
      connectWithKey(storedKey)
    }
  }, [])

  const connectWithKey = async (key: string) => {
    try {
      setStatus('connecting')
      const client = new NostrClient(key, { relayUrl: wsUrl })
      await client.connect()
      console.log('Successfully connected to relay')
      // Store the key only after successful connection
      localStorage.setItem('nostr_key', key)
      onComplete(client)
    } catch (e) {
      console.error('Connection failed:', e)
      let errorMessage = 'Failed to connect';
      if (e instanceof Error) {
        if (e.message.includes('timeout')) {
          errorMessage = 'Connection timed out. Please check your network and try again.';
        } else if (e.message.includes('auth failed')) {
          errorMessage = 'Authentication failed. Please check your key and try again.';
        } else if (e.message.includes('Main relay')) {
          errorMessage = `Cannot connect to the groups relay at ${wsUrl}. Please try again.`;
        } else {
          errorMessage = e.message;
        }
      }
      setError(new Error(errorMessage))
      setStatus('idle')
      // Clear stored key if connection fails
      localStorage.removeItem('nostr_key')
    }
  }

  if (error) {
    return <ErrorState error={error} onRetry={() => setError(null)} />
  }

  if (status === 'connecting') {
    return (
      <LoadingState
        title="Connecting"
        message="Establishing connection to relay..."
      />
    )
  }

  return <AuthPrompt onSubmit={connectWithKey} />
}

const Root = () => {
  const [client, setClient] = useState<NostrClient | null>(null)

  const handleLogout = () => {
    if (client) {
      client.disconnect()
    }
    setClient(null)
    // Clear stored key on explicit logout
    localStorage.removeItem('nostr_key')
  }

  if (!client) {
    return <Initialization onComplete={setClient} />
  }

  return <App client={client} onLogout={handleLogout} />
}

render(<Root />, document.getElementById('app')!)

