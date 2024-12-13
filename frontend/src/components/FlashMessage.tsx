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

    const bgColor = {
      success: 'bg-green-500',
      error: 'bg-red-500',
      info: 'bg-blue-500'
    }[type]

    return (
      <div class={`fixed top-0 left-0 right-0 z-50 flex items-center justify-center transition-transform duration-300 ${message ? 'translate-y-0' : '-translate-y-full'}`}>
        <div class={`${bgColor} text-white px-6 py-3 rounded-b-lg shadow-lg flex items-center gap-2 max-w-2xl mx-auto`}>
          <span class="text-sm font-medium">{message}</span>
          <button
            onClick={this.props.onDismiss}
            class="ml-2 text-white hover:text-gray-200 transition-colors"
            aria-label="Dismiss message"
          >
            ✕
          </button>
        </div>
      </div>
    )
  }
}