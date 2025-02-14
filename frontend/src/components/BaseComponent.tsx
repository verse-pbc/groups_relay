import { Component, VNode } from 'preact'
import { NostrGroupError } from '../api/nostr_client'

interface BaseComponentProps {
  showMessage: (message: string, type: 'success' | 'error' | 'info') => void
}

export class BaseComponent<P extends BaseComponentProps, S = {}> extends Component<P, S> {
  protected showError = (prefix: string, error: unknown) => {
    console.error(prefix, error);
    const message = error instanceof NostrGroupError ? error.displayMessage : String(error);
    this.props.showMessage(`${prefix}: ${message}`, 'error');
  }

  render(): VNode | null {
    return null;
  }
}