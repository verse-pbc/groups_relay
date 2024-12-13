import 'preact/debug'
import 'preact/devtools'
import { render } from 'preact'
import { NostrClient } from './api/nostr_client.ts'
import { App } from './components/App.tsx'
import './style.css'

// TODO: This is the same test key used in the backend. We could do some nip07 in the future?
const RELAY_SECRET_KEY = "6b911fd37cdf5c81d4c0adb1ab7fa822ed253ab0ad9aa18d77257c88b29b718e"
const client = new NostrClient(RELAY_SECRET_KEY, {
  relayUrl: "ws://127.0.0.1:8080"
})

client.connect().then(() => {
  render(<App client={client} />, document.getElementById('app')!)
}).catch(console.error)

