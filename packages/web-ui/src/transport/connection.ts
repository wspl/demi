/**
 * What decides whether the page reaches the backend: the browser has a
 * network, the backend did not say it shuts down, and the synchronization
 * channel's lost connection was not refused twice.
 */
export interface ConnectionFacts {
  /** The browser reports a network. */
  online: boolean
  /** The channel closed with 1001 `backend_closing`, and no snapshot came since. */
  restarting: boolean
  /** Connections of the channel that ended before their snapshot, in a row. */
  failedAttempts: number
  /** The page has a copy of the state to keep showing; before its first, the regions show their failure. */
  hasSnapshot: boolean
}

/** Where the page's connection to the backend stands. */
export type ConnectionState = 'online' | 'reconnecting' | 'offline'

/** Why the connection stands where it does, for the log. */
export type ConnectionCause = 'browser_offline' | 'backend_restarting' | 'channel_failed' | 'reached'

/**
 * Where the connection stands.
 *
 * - No network wins, since nothing else can be true of it: `offline`.
 * - A backend that said it restarts, or a lost channel whose first new attempt
 *   failed as well, is `reconnecting`; a blip that the next connection mends
 *   is never one.
 */
export function connectionState(facts: ConnectionFacts): ConnectionState {
  if (!facts.online) {
    return 'offline'
  }
  return facts.restarting || (facts.hasSnapshot && facts.failedAttempts >= 2) ? 'reconnecting' : 'online'
}

/** What made the connection stand where `connectionState` says. */
export function connectionCause(facts: ConnectionFacts): ConnectionCause {
  if (!facts.online) {
    return 'browser_offline'
  }
  if (facts.restarting) {
    return 'backend_restarting'
  }
  return facts.failedAttempts > 0 ? 'channel_failed' : 'reached'
}

/** Whether the page cannot reach the backend. */
export function backendUnreachable(facts: ConnectionFacts): boolean {
  return connectionState(facts) !== 'online'
}
