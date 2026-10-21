import { computed, type ComputedRef } from 'vue'
import type { SessionPhase } from '@demicodes/protocol'
import { activitySlotKind, type ActivitySlotInput } from './activity-slot'
import { turnStartingMessage } from './send-way'
import type { AgentActivity } from './agent-activity'
import type { PendingSubmissionState } from './types'
import { pressOf } from './wait-clock'

export type ActivitySlotState =
  | { kind: 'connecting' }
  | {
    kind: 'requesting'
    /**
     * When the row's current wait began, as `Date.now()`; the Requesting
     * clock counts from it.
     */
    since: number
  }

export interface ActivitySlotSources {
  /**
   * What the row's kind is decided from, except what the agent does and
   * whether a message being delivered starts a turn.
   */
  input: () => Omit<ActivitySlotInput, 'startingTurn' | 'waiting' | 'actionSent'>
  phase: () => SessionPhase
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSubmissions: () => readonly PendingSubmissionState[]
  activity: () => AgentActivity
}

/**
 * The transcript's tail row: why it waits, and since when.
 *
 * The row says Requesting while the server says the agent waits, or while an
 * action of this page's own is on its way (`activitySlotKind`). A message
 * sent while the conversation is idle starts a turn, so the row says
 * Requesting from the press, through its delivery and the server's
 * confirmation, until the answer begins: one wait, one word and one clock.
 * The press is the conversation's (`wait-clock.ts`), and the server's wait
 * names the action that began it, so a list drawn again, or a wait told
 * again after a reconnect, counts on from the press. A wait no press of this
 * page began counts from the server's start. Which message on its way
 * starts a turn is `turnStartingMessage`'s, which the composer's Stop reads
 * too, so the two show together. What follows the wait, a step, the answer
 * or a Stop, takes the row's place and rolls over it (`succeedKeys`,
 * `RowRoll`).
 */
export function useActivitySlot(sources: ActivitySlotSources): {
  slot: ComputedRef<ActivitySlotState | null>
  /**
   * A message on its way starts a turn: it is the user's next message already.
   */
  startingTurn: ComputedRef<boolean>
} {
  const { input, phase, pendingSubmissions, activity } = sources

  // A message on its way that waits for the backend to be reachable again
  // still starts its turn: the row says Connecting meanwhile, or the
  // connection banner says the backend is away, and Requesting once the
  // socket is back, counting from the press.
  const starting = computed(() => turnStartingMessage(phase(), pendingSubmissions()))
  const startingTurn = computed(() => starting.value !== undefined)
  const kind = computed(() => activitySlotKind({
    ...input(),
    startingTurn: startingTurn.value,
    waiting: activity().waitingSince !== null,
    actionSent: activity().sentAction !== null,
  }))

  // An action on its way counts from its press; then the server's wait
  // counts from the press of the action that began it, if this page pressed
  // it, or from the server's start.
  const since = computed(() => {
    const { waitingSince, waitAction, waitClock, sentAction } = activity()
    const onItsWay = starting.value?.id ?? sentAction
    const pressedAt = onItsWay === null ? undefined : pressOf(waitClock, onItsWay)
    if (pressedAt !== undefined) {
      return pressedAt
    }
    if (waitingSince === null) {
      return null
    }
    const waitPressed = waitAction === null ? undefined : pressOf(waitClock, waitAction)
    return waitPressed === undefined ? waitingSince : Math.min(waitPressed, waitingSince)
  })

  const slot = computed<ActivitySlotState | null>(() => {
    switch (kind.value) {
      case 'connecting':
        return { kind: 'connecting' }
      case 'requesting':
        // Each action of the page's is pressed as it is sent; one whose
        // press the page does not hold, while the server tells no wait,
        // waits for nothing the row could count.
        return since.value === null ? null : { kind: 'requesting', since: since.value }
      case null:
        return null
    }
  })

  return { slot, startingTurn }
}
