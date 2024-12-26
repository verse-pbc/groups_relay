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
      success: 'bg-green-500/10 text-green-500 border-green-500/20',
      error: 'bg-red-500/10 text-red-500 border-red-500/20',
      info: 'bg-accent/10 text-accent border-accent/20'
    }[type]

    return (
      <div class="fixed top-4 left-1/2 -translate-x-1/2 z-50 w-full max-w-xl mx-auto px-4">
        <div class={`${styles} px-4 py-3 rounded-lg shadow-lg border flex items-center justify-between`}>
          <span class="text-sm font-medium">{message}</span>
          <button
            onClick={this.props.onDismiss}
            class="ml-3 text-current opacity-60 hover:opacity-100 transition-opacity"
            aria-label="Dismiss message"
          >
            ×
          </button>
        </div>
      </div>
    )
  }
}