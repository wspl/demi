import { computed, ref, watch, type ComputedRef } from 'vue'
import { activitySlotKind, type ActivityKind, type ActivitySlotInput } from './activity-slot'
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
 * (`product.md` § Recovering an unfinished turn). A message sent during an
 * action joins the queue instead, so whether a message starts a turn is read
 * from the phase at the moment it is sent, and again when Retry sends it anew.
 * A failed delivery ends that wait. What follows the wait, a step, the
 * answer or a Stop, takes the row's place and rolls over it
 * (`succeedKeys`, `RowRoll`).
 */
export function useActivitySlot(sources: ActivitySlotSources): {
  slot: ComputedRef<ActivitySlotState | null>
  /** A message on its way starts a turn: it is the user's next message already. */
  startingTurn: ComputedRef<boolean>
} {
  const { input, pendingSubmissions, waitingSince, scope } = sources

  // The message on its way that starts a turn; null when none is. Read when
  // the message is sent, before the server's phase can answer it.
  const startingTurnId = ref<string | null>(null)
  watch(
    () => deliveringId(pendingSubmissions()),
    (id) => {
      startingTurnId.value = id !== null && input().phase === 'idle' ? id : null
    },
    { immediate: true, flush: 'sync' },
  )
  const startingTurn = computed(() => startingTurnId.value !== null)
  const kind = computed(() =>
    activitySlotKind({ ...input(), startingTurn: startingTurn.value }),
  )

  // The clock counts from when the wait began, which is the session's fact,
  // not this view's (`runtime.md` § Waiting for the model): a view shown
  // during a wait counts from the start the server tells, and runs on across
  // the confirmation of a message and the agent's own retries alike. A wait
  // this page started, by sending a message or a recovery, counts from that
  // action, before the server has it. Until the server tells a start, the row
  // counts from when it began to say Requesting.
  const requesting = computed(() => kind.value === 'requesting')
  const shownSince = ref(Date.now())
  const startedHere = ref(false)
  watch(
    () => [requesting.value, scope()] as const,
    ([isRequesting, currentScope], [wasRequesting, previousScope]) => {
      if (isRequesting && (!wasRequesting || currentScope !== previousScope)) {
        shownSince.value = Date.now()
        startedHere.value = startingTurnId.value !== null || input().recovering
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

/**
 * The id of the first message on its way sent as the next message, which
 * is the one that can start a turn; a steer or a queued message starts
 * none, and a failed delivery is not on its way. One that waits for the
 * backend to be reachable again is: the row says Connecting meanwhile, or
 * the connection banner says the backend is away, and Requesting once the
 * socket is back.
 */
function deliveringId(submissions: readonly PendingSubmissionState[]): string | null {
  return submissions.find((submission) => submission.way === 'send' && submission.error === null)?.id ?? null
}
