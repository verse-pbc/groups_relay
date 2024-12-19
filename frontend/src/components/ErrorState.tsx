import { FunctionComponent } from 'preact'

interface ErrorStateProps {
  error: Error | string
  onRetry?: () => void
}

export const ErrorState: FunctionComponent<ErrorStateProps> = ({ error, onRetry }) => {
  const errorMessage = error instanceof Error ? error.message : error
  const errorStack = error instanceof Error ? error.stack : ''

  return (
    <div class="p-5 text-red-500">
      <h2 class="text-xl font-bold mb-2">Connection Error</h2>
      <p class="mb-4">{errorMessage}</p>
      {errorStack && (
        <pre class="mt-2 p-2 bg-red-50 rounded text-sm overflow-auto">
          {errorStack}
        </pre>
      )}
      {onRetry && (
        <button
          onClick={onRetry}
          class="mt-5 px-4 py-2 bg-accent text-white rounded hover:bg-accent/90 transition-colors"
        >
          Try Again
        </button>
      )}
    </div>
  )
}