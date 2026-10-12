import { computed, ref, watch, type ComputedRef } from 'vue'
import { activitySlotKind, type ActivityKind, type ActivitySlotInput } from './activity-slot'
import { turnStartingMessage } from './send-way'
import type { PendingSubmissionState } from './types'

export interface ActivitySlotState {
  kind: ActivityKind
  /** When the row's current wait began, as `Date.now()`; the Requesting clock counts from it. */
  since: number
}

export interface ActivitySlotSources {
  /** What the row's kind is decided from, except whether a message being delivered starts a turn. */
  input: () => Omit<ActivitySlotInput, 'startingTurn'>
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSubmissions: () => readonly PendingSubmissionState[]
  /** When the agent began to wait for the model's answer, as the server tells it; null when it waits for none. */
  waitingSince: () => number | null
  /** The conversation the list shows; another one starts over. */
  scope: () => string
}

/**
 * The transcript's tail row: why it waits, and since when.
 *
 * A message sent while the conversation is idle starts a turn, so the row
 * says Requesting from the send, through its delivery and the server's
 * confirmation, until the answer begins: one wait, one word and one clock
 * (`product.md` § Recovering an unfinished turn). Which message on its way
 * starts a turn is `turnStartingMessage`'s, which the composer's Stop reads
 * too, so the two show together. A failed delivery ends that wait. What
 * follows the wait, a step, the answer or a Stop, takes the row's place and
 * rolls over it (`succeedKeys`, `RowRoll`).
 */
export function useActivitySlot(sources: ActivitySlotSources): {
  slot: ComputedRef<ActivitySlotState | null>
  /** A message on its way starts a turn: it is the user's next message already. */
  startingTurn: ComputedRef<boolean>
} {
  const { input, pendingSubmissions, waitingSince, scope } = sources

  // A message on its way that waits for the backend to be reachable again
  // still starts its turn: the row says Connecting meanwhile, or the
  // connection banner says the backend is away, and Requesting once the
  // socket is back.
  const startingTurn = computed(() => turnStartingMessage(input().phase, pendingSubmissions()) !== undefined)
  const kind = computed(() =>
    activitySlotKind({ ...input(), startingTurn: startingTurn.value }),
  )

  // The clock counts from when the wait began, which is the session's fact,
  // not this view's (`runtime.md` § Waiting for the model): the server tells
  // a start for as long as the row says Requesting, from the turn's start
  // or a step's end until something of the answer shows, so a view shown
  // during a wait counts from it, and runs on across the confirmation of a
  // message and the agent's own retries alike. A wait this page started, by
  // sending a message or a recovery, counts from that action, before the
  // server has it. Until the server's start arrives, the row counts from
  // when it began to say Requesting.
  const requesting = computed(() => kind.value === 'requesting')
  const shownSince = ref(Date.now())
  const startedHere = ref(false)
  watch(
    () => [requesting.value, scope()] as const,
    ([isRequesting, currentScope], [wasRequesting, previousScope]) => {
      if (isRequesting && (!wasRequesting || currentScope !== previousScope)) {
        shownSince.value = Date.now()
        startedHere.value = startingTurn.value || input().recovering
      }
    },
  )
  const since = computed(() => {
    const told = waitingSince()
    if (told === null) {
      return shownSince.value
    }
    return startedHere.value ? Math.min(told, shownSince.value) : told
  })

  const slot = computed<ActivitySlotState | null>(() =>
    kind.value ? { kind: kind.value, since: since.value } : null,
  )

  return { slot, startingTurn }
}
