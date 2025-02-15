import { Component } from 'preact'
import type { Group } from '../types'
import { GroupTimestamps } from './GroupTimestamps'

interface InfoSectionProps {
  group: Group
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

interface InfoSectionState {
  copiedId: boolean
}

export class InfoSection extends Component<InfoSectionProps, InfoSectionState> {
  private copyTimeout: number | null = null;

  state = {
    copiedId: false
  }

  componentWillUnmount() {
    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }
  }

  copyGroupId = () => {
    navigator.clipboard.writeText(this.props.group.id)
    this.setState({ copiedId: true })

    if (this.copyTimeout) {
      window.clearTimeout(this.copyTimeout)
    }

    this.copyTimeout = window.setTimeout(() => {
      this.setState({ copiedId: false })
    }, 2000)
  }

  render() {
    const { group } = this.props
    const { copiedId } = this.state

    return (
      <div class="space-y-4">
        {/* Group ID */}
        <div class="space-y-1">
          <label class="block text-sm font-medium text-[var(--color-text-secondary)]">
            Group ID
          </label>
          <div class="flex items-center gap-2">
            <code class="flex-1 px-2 py-1 text-sm bg-[var(--color-bg-primary)] rounded font-mono">
              {group.id}
            </code>
            <button
              onClick={this.copyGroupId}
              class="text-xs text-[var(--color-text-tertiary)] hover:text-[var(--color-text-secondary)] transition-colors"
            >
              {copiedId ? 'Copied!' : 'Copy'}
            </button>
          </div>
        </div>

        <GroupTimestamps group={group} />
      </div>
    )
  }
} 