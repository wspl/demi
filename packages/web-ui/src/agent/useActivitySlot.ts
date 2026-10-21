import { computed, type ComputedRef } from 'vue'
import type { SessionPhase } from '@demicodes/protocol'
import { activitySlotKind, type ActivitySlotInput } from './activity-slot'
import { turnStartingMessage } from './send-way'
import type { AgentActivity } from './agent-activity'
import type { PendingSubmissionState } from './types'

export type ActivitySlotState =
  | { kind: 'connecting' }
  | {
    kind: 'requesting'
    /**
     * When the server began the wait, as `Date.now()`, which the row counts
     * from; null while the page's own action has no answer yet, when the row
     * shows no time.
     */
    since: number | null
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
 * action of this page's own is on its way (`activitySlotKind`). Only the
 * server counts: while the page's action has no answer, the row shows no
 * time, and once the server tells the wait, the row counts from the wait's
 * start the server told, so a list drawn again, a reload or a reconnect
 * shows the same count. Which message on its way starts a turn is
 * `turnStartingMessage`'s, which the composer's Stop reads too, so the two
 * show together. A step or a Stop that follows the wait takes the row's place
 * and rolls over it (`succeedKeys`, `RowRoll`); the answer, a failure or
 * anything else appears in its place.
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
  // socket is back.
  const startingTurn = computed(() => turnStartingMessage(phase(), pendingSubmissions()) !== undefined)
  const kind = computed(() => activitySlotKind({
    ...input(),
    startingTurn: startingTurn.value,
    waiting: activity().waitingSince !== null,
    actionSent: activity().actionSent,
  }))

  const slot = computed<ActivitySlotState | null>(() => {
    switch (kind.value) {
      case 'connecting':
        return { kind: 'connecting' }
      case 'requesting':
        return { kind: 'requesting', since: activity().waitingSince }
      case null:
        return null
    }
  })

  return { slot, startingTurn }
}
