import { FunctionComponent } from 'preact'

interface LoadingStateProps {
  title: string
  message: string
}

export const LoadingState: FunctionComponent<LoadingStateProps> = ({ title, message }) => (
  <div class="p-5 text-primary">
    <h2 class="text-xl font-bold mb-2">{title}</h2>
    <p>{message}</p>
  </div>
)