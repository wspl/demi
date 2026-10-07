import type { SentenceText } from '../ui/ui-text'

/**
 * Why the page cannot reach the backend, as its connection banner says
 * (`web-application.md` § A page of another build): the browser has no
 * network, the backend said it shuts down and will be back, or a lost
 * synchronization channel's first new attempt failed too.
 */
export type ConnectionProblem = 'offline' | 'restarting' | 'reconnecting'

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
 * What the banner says, or null while the page reaches the backend. No
 * network wins, since nothing else can be true of it; a lost channel shows
 * nothing until its first new attempt failed as well, so a blip that the
 * next connection mends never flashes a banner.
 */
export function connectionProblem(facts: ConnectionFacts): ConnectionProblem | null {
  if (!facts.online) {
    return 'offline'
  }
  if (facts.restarting) {
    return 'restarting'
  }
  return facts.hasSnapshot && facts.failedAttempts >= 2 ? 'reconnecting' : null
}

/** The banner's words for each problem. */
export const CONNECTION_WORDS: Record<ConnectionProblem, SentenceText> = {
  offline: 'You\'re offline',
  restarting: 'Demi is restarting',
  reconnecting: 'Reconnecting to Demi',
}
