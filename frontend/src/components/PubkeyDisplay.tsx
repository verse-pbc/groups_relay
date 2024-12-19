import { Component } from 'preact'

interface PubkeyDisplayProps {
  pubkey: string
  showAvatar?: boolean
  showCopy?: boolean
}

interface PubkeyDisplayState {
  copied: boolean
}

export class PubkeyDisplay extends Component<PubkeyDisplayProps, PubkeyDisplayState> {
  state = {
    copied: false
  }

  private copyTimeout: number | null = null

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }
  }

  handleCopy = () => {
    navigator.clipboard.writeText(this.props.pubkey)
    this.setState({ copied: true })

    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }

    this.copyTimeout = window.setTimeout(() => {
      this.setState({ copied: false })
    }, 2000)
  }

  render() {
    const { pubkey, showAvatar = false, showCopy = true } = this.props
    const { copied } = this.state

    return (
      <div class="flex items-center gap-2">
        {showAvatar && (
          <div class="shrink-0 h-8 w-8 rounded-full bg-accent/10 flex items-center justify-center">
            <span class="text-sm font-medium text-accent">
              {pubkey.slice(0, 2).toUpperCase()}
            </span>
          </div>
        )}
        <button
          onClick={showCopy ? this.handleCopy : undefined}
          title={showCopy ? pubkey : undefined}
          class={`text-xs font-mono ${showCopy
            ? 'hover:text-accent cursor-pointer'
            : 'cursor-default'
          } transition-colors flex items-center gap-1`}
        >
          <span class="text-[var(--color-text-primary)]">
            {pubkey.slice(0, 8)}...
          </span>
          {showCopy && (
            <span class="text-xs text-[var(--color-text-tertiary)]">
              {copied ? '✓' : '📋'}
            </span>
          )}
        </button>
      </div>
    )
  }
}