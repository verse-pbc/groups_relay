import { Component } from 'preact'

export interface FlashMessageProps {
  message: string | null
  type?: 'success' | 'error' | 'info'
  onDismiss: () => void
}

export class FlashMessage extends Component<FlashMessageProps> {
  componentDidUpdate(prevProps: FlashMessageProps) {
    if (this.props.message && !prevProps.message) {
      setTimeout(() => {
        this.props.onDismiss()
      }, 5000)
    }
  }

  render() {
    const { message, type = 'info' } = this.props
    if (!message) return null

    const styles = {
      success: 'bg-green-500/20 text-green-600 border-green-500/30',
      error: 'bg-red-500/20 text-red-600 border-red-500/30',
      info: 'bg-accent/20 text-accent border-accent/30'
    }[type]

    return (
      <div class="fixed top-4 left-1/2 -translate-x-1/2 z-50 w-full max-w-xl mx-auto px-4">
        <div class={`${styles} px-4 py-3 rounded-lg shadow-xl border backdrop-blur-sm flex items-center justify-between`}>
          <span class="text-sm font-medium">{message}</span>
          <button
            onClick={this.props.onDismiss}
            class="ml-3 text-current opacity-70 hover:opacity-100 transition-opacity"
            aria-label="Dismiss message"
          >
            ×
          </button>
        </div>
      </div>
    )
  }
}