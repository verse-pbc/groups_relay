interface Window {
  nostr?: {
    getPublicKey(): Promise<string>;
    signEvent(event: any): Promise<any>;
    // Add other NIP-07 methods as needed
  };
}
