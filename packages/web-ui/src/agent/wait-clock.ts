/**
 * The presses of this page's own actions, by the action's id, as the
 * conversation, or a side question, holds them.
 *
 * A message, a recovery and a side question's question, follow-up or Retry
 * each go under an id the page names. When the action starts as the server
 * admits it, the wait for the model it begins carries that id (the agent
 * entry's `waitAction`), on every delivery of the entry. The transcript's
 * tail row counts that wait from the user's press, held here, before the
 * server has the action and after; a wait no press of this page began counts
 * from the server's start (`useActivitySlot`).
 *
 * Whoever acts records the press, whether a view shows the conversation or
 * not, so a list drawn again counts on from it. Nothing pairs a press with a
 * wait by when either began: the id is the wait's identity.
 *
 * ```text
 * press ──► pressed ──► the server's wait carries the id ──► counted from
 *   │                                                          the press
 *   └── refused ──► forgotten
 * ```
 */

/** Where a conversation, or a side question, holds the presses of its page's actions. */
export interface WaitClock {
  /** When the user pressed each action, as `Date.now()`, by the action's id. */
  presses: Record<string, number>
}

/** A clock before the page pressed anything. */
export function waitClock(): WaitClock {
  return { presses: {} }
}

/** The user pressed the action `id`. */
export function pressed(clock: WaitClock, id: string, at = Date.now()): void {
  clock.presses[id] = at
}

/** The server refused the action `id`: no wait of it comes. */
export function refused(clock: WaitClock, id: string): void {
  delete clock.presses[id]
}

/** When the user pressed the action `id`; undefined for one this page did not press. */
export function pressOf(clock: WaitClock | undefined, id: string): number | undefined {
  return clock?.presses[id]
}

/**
 * The server tells the wait the action `id` began, or a wait no action
 * began, or none, for null. The presses before a told action's press are of
 * waits that are over, or of actions that began none, as a steer: they go,
 * so the clock holds a few presses however long the conversation runs.
 */
export function serverWaits(clock: WaitClock, id: string | null): void {
  const told = id === null ? undefined : clock.presses[id]
  if (told === undefined) {
    return
  }
  for (const [each, at] of Object.entries(clock.presses)) {
    if (at < told) {
      delete clock.presses[each]
    }
  }
}
