import { computed, watch, type ComputedRef } from 'vue'
import type { SessionPhase } from '@demicodes/protocol'
import { pageLog } from '../infra/log'
import { activitySlotKind, type ActivitySlotInput } from './activity-slot'
import { turnStartingMessage } from './send-way'
import type { AgentActivity } from './agent-activity'
import type { PendingSubmissionState } from './types'

export interface ActivitySlotState {
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
  /** The conversation and agent the row is of, which its lines are written under. */
  scope?: () => { conversation: string | undefined; node: string | null }
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
  const { input, phase, pendingSubmissions, activity, scope } = sources

  // A message on its way that waits for the lane still starts its turn: the
  // row says Requesting from the send.
  const startingTurn = computed(() => turnStartingMessage(phase(), pendingSubmissions()) !== undefined)
  const kind = computed(() => activitySlotKind({
    ...input(),
    startingTurn: startingTurn.value,
    waiting: activity().waitingSince !== null,
    actionSent: activity().actionSent,
  }))

  const slot = computed<ActivitySlotState | null>(() =>
    kind.value === null ? null : { kind: 'requesting', since: activity().waitingSince })

  logTimer(slot, scope)

  return { slot, startingTurn }
}

/**
 * Writes the row's count as the log tells it: where the count started from, the
 * server's word (`since`) or this page's own action (none yet), and every
 * change of that start, so a count that restarted says why. `at: 'view'` is
 * the row a view that opened already found waiting.
 */
function logTimer(slot: ComputedRef<ActivitySlotState | null>, scope: ActivitySlotSources['scope']): void {
  let startedAt: number | null = null
  watch(slot, (now, before) => {
    const where = scope?.()
    const fields = { timer: 'requesting', ...(where?.node ? { node: where.node } : {}) }
    const conversation = where?.conversation
    if (now !== null && !before) {
      startedAt = performance.now()
      pageLog.info('timer.started', {
        ...fields,
        since: now.since === null ? 'page_action' : 'server',
        elapsed_ms: now.since === null ? 0 : Math.max(0, Date.now() - now.since),
        at: before === undefined ? 'view' : 'change',
      }, conversation)
    } else if (now !== null && before !== null && before !== undefined && now.since !== before.since) {
      pageLog.info('timer.reset', {
        ...fields,
        from_since: before.since,
        to_since: now.since,
        cause: before.since === null ? 'server_wait' : now.since === null ? 'action_sent' : 'server_new_wait',
      }, conversation)
    } else if (now === null && before) {
      pageLog.info('timer.ended', { ...fields, ms: startedAt === null ? 0 : Math.round(performance.now() - startedAt) }, conversation)
      startedAt = null
    }
  }, { immediate: true })
}
