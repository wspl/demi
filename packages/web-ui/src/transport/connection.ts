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

/**
 * Whether the page cannot reach the backend.
 *
 * - No network wins, since nothing else can be true of it.
 * - A lost channel counts only once its first new attempt failed as well, so a
 *   blip that the next connection mends is never unreachable.
 */
export function backendUnreachable(facts: ConnectionFacts): boolean {
  return !facts.online || facts.restarting || (facts.hasSnapshot && facts.failedAttempts >= 2)
}
