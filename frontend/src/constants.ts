// UI and Timing Constants
export const TIMEOUTS = {
  COMPATIBILITY_CHECK_DELAY: 100, // ms delay before checking recipient compatibility
  NUTZAP_SEND_TIMEOUT: 15000, // ms timeout for nutzap sending operations
  RELAY_CONNECTION_TIMEOUT: 3000, // ms timeout for relay connections
  COPY_FEEDBACK_DURATION: 2000, // ms duration for copy success feedback
  BALANCE_THROTTLE: 1000, // ms throttle for balance calculations
} as const;

export const CACHE_DURATIONS = {
  BALANCE_CACHE_TIMEOUT: 5 * 60 * 1000, // 5 minutes
  TRANSACTION_HISTORY_LIMIT: 100, // max transactions to keep
} as const;

export const COLORS = {
  BITCOIN_ORANGE: '#f7931a',
  BITCOIN_ORANGE_HOVER: '#f68e0a',
  BITCOIN_ORANGE_10: '#f7931a1a',
  BITCOIN_ORANGE_20: '#f7931a33',
} as const;

export const MIN_NUTZAP_AMOUNT = 1; // minimum sats for nutzap compatibility check