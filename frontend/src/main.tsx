import 'preact/debug'
import 'preact/devtools'
import { render } from 'preact'
import { NostrClient } from './api/nostr_client.ts'
import { App } from './components/App.tsx'
import './style.css'

// TODO: This is the same test key used in the backend. We could do some nip07 in the future?
const RELAY_SECRET_KEY = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e"

// In development, connect directly to the relay
// In production, use the WebSocket proxy
const wsUrl = import.meta.env.DEV
  ? "ws://127.0.0.1:8080"
  : `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}`

const client = new NostrClient(RELAY_SECRET_KEY, { relayUrl: wsUrl })

client.connect()
  .then(() => {
    console.log('Successfully connected to relay')
    render(<App client={client} />, document.getElementById('app')!)
  })
  .catch(error => {
    console.error('Connection error:', error)
    const app = document.getElementById('app')
    if (app) {
      app.innerHTML = `<div style="color: red; padding: 20px;">
        <h2>Connection Error</h2>
        <p>${error.message}</p>
        <pre>${error.stack}</pre>
      </div>`
    }
  })

