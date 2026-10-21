import type { SessionLoad } from './session-status'

/**
 * Why the transcript's tail row is waiting.
 *
 * - `connecting` is the socket, whether it is lost or not made yet.
 * - `requesting` is the wait for the provider's answer. It runs from the send
 *   of a message that starts a turn, its delivery included. A recovery the
 *   server has not acknowledged yet and the agent's own retries are the same
 *   wait and keep the same word.
 */
export type ActivityKind = 'connecting' | 'requesting'

/**
 * An action the client sent whose acknowledgement (the next `phase` event)
 * has not arrived. The tail row says Requesting meanwhile, with no time.
 */
export type PendingAction = 'resume' | null

export interface ActivitySlotInput {
  load: SessionLoad
  /**
   * The page cannot reach the backend, and the host's connection banner says
   * so. A conversation whose socket waits for the same backend leaves that to
   * the banner.
   */
  backendAway: boolean
  /**
   * An action of this page's own other than a message, a recovery or a side
   * question's question, follow-up or Retry, is on its way: the server has
   * not answered it yet.
   */
  actionSent: boolean
  /**
   * A message sent to start a turn is on its way to Demi: the wait for its
   * answer has begun, though the server has confirmed neither the message nor
   * the turn yet.
   */
  startingTurn: boolean
  /** The server says the agent waits for the model's answer. */
  waiting: boolean
}

/**
 * The tail row's kind, or null when nothing waits.
 *
 * - The row says Requesting only while the server says the agent waits, or
 *   while an action of this page's own, a message that starts a turn, a
 *   recovery or a side question's, is on its way to the server.
 *   Nothing in the transcript makes a wait: a transcript the page holds only
 *   in part, as on a first opening or from an offline copy, would make one
 *   up.
 * - A lost socket wins over everything, since a recovery or a turn cannot
 *   progress without it. The row says Connecting while the page still reaches
 *   the backend, and nothing while the connection banner says the backend is
 *   away.
 * - A conversation that failed to open waits for nothing. The failure told at
 *   the tail says so.
 * - A first opening whose socket is not made yet says Connecting as a lost one
 *   does, also while it shows the copy the page kept of the conversation,
 *   unless a message sent to start a turn is on its way: Connecting is for
 *   opening a conversation, not for sending in it, so the row says Requesting
 *   from the send, as the first message of a new conversation does while its
 *   socket opens
 *   (`docs/decisions/0045-a-new-conversation-sends-with-its-creation.md`).
 *   A socket that could not be made, `connecting`, still says Connecting.
 */
export function activitySlotKind(input: ActivitySlotInput): ActivityKind | null {
  if (input.load === 'reconnecting' || input.load === 'connecting' || input.load === 'opening') {
    if (input.backendAway) {
      return null
    }
    return input.load === 'opening' && input.startingTurn ? 'requesting' : 'connecting'
  }
  if (input.load === 'failed') {
    return null
  }
  return input.actionSent || input.startingTurn || input.waiting ? 'requesting' : null
}
