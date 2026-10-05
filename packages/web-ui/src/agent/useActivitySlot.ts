import { computed, onScopeDispose, ref, watch, type ComputedRef, type Ref } from 'vue'
import {
  ACTIVITY_HANDOFF_MS,
  activitySlotKind,
  isHandoffBlock,
  type ActivityKind,
  type ActivitySlotInput,
  type HandoffBlock,
} from './activity-slot'
import type { MessageListBlock } from './pending-steers'
import type { PendingSubmissionState } from './types'

export interface ActivitySlotState {
  kind: ActivityKind
  /** The block rolling into the row; the row shows its face until the transcript takes it. */
  incoming: HandoffBlock | null
  /** When the row's current wait began, as `Date.now()`; the Requesting clock counts from it. */
  since: number
}

export interface ActivitySlotSources {
  /** What the row's kind is decided from, except whether a message being delivered starts a turn. */
  input: () => Omit<ActivitySlotInput, 'startingTurn'>
  /** The message sent and not yet confirmed. */
  pendingSubmission: () => PendingSubmissionState | null
  /** The last block the list shows of the transcript: the one that may roll into the row. */
  tail: () => MessageListBlock | undefined
  /** The conversation the list shows; another one starts over. */
  scope: () => string
}

interface Hold {
  id: string
  /** The face the row showed when the block arrived; it stays the row's kind until the handoff ends. */
  kind: ActivityKind
}

/**
 * The transcript's tail row: why it waits, since when, and the block rolling
 * into it.
 *
 * A message sent while the conversation is idle starts a turn, so the row
 * says Requesting from the send, through its delivery and the server's
 * confirmation, until the answer begins: one wait, one word and one clock
 * (`product.md` § Recovering an unfinished turn). A message sent during an
 * action joins the queue instead, so whether a message starts a turn is read
 * from the phase at the moment it is sent, and again when Retry sends it anew.
 * A failed delivery ends that wait.
 *
 * A thinking or tool block that arrives while the row is waiting does not
 * replace the row: it rolls in as the row's face, and after the roll the
 * transcript row takes over in the same place. `heldId` is the block the list
 * leaves out meanwhile. The hold ends on its timer, when the block leaves the
 * tail (withdrawn or followed by another), when the scope changes, and on
 * unmount; every path releases through one function.
 */
export function useActivitySlot(sources: ActivitySlotSources): {
  heldId: ComputedRef<string | null>
  slot: ComputedRef<ActivitySlotState | null>
} {
  const { input, pendingSubmission, tail, scope } = sources

  // The message on its way that starts a turn; null when none is. Read when
  // the message is sent, before the server's phase can answer it.
  const startingTurnId = ref<string | null>(null)
  watch(
    () => deliveringId(pendingSubmission()),
    (id) => {
      startingTurnId.value = id !== null && input().phase === 'idle' ? id : null
    },
    { immediate: true, flush: 'sync' },
  )
  const baseKind = computed(() =>
    activitySlotKind({ ...input(), startingTurn: startingTurnId.value !== null }),
  )

  const hold: Ref<Hold | null> = ref(null)
  let timer: ReturnType<typeof setTimeout> | undefined

  function release(): void {
    if (timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
    hold.value = null
  }

  function start(id: string, kind: ActivityKind): void {
    release()
    hold.value = { id, kind }
    timer = setTimeout(() => {
      timer = undefined
      hold.value = null
    }, ACTIVITY_HANDOFF_MS)
  }

  watch(
    () => [tail()?.id, baseKind.value] as const,
    ([tailId], [previousTailId, previousKind]) => {
      if (hold.value && hold.value.id !== tailId) {
        release()
      }
      if (tailId === previousTailId || !previousKind) {
        return
      }
      const block = tail()
      if (isHandoffBlock(block)) {
        start(block.id, previousKind)
      }
    },
  )

  watch(scope, release)
  onScopeDispose(release)

  const heldId = computed(() => hold.value?.id ?? null)
  const face = computed(() => {
    const current = hold.value
    if (current) {
      const block = tail()
      const incoming = isHandoffBlock(block) && block.id === current.id ? block : null
      return { kind: current.kind, incoming }
    }
    const kind = baseKind.value
    return kind ? { kind, incoming: null } : null
  })

  // The clock starts when the row begins to wait for the provider, and runs on
  // while it keeps waiting: across the confirmation of a message and the
  // agent's own retries alike.
  const requesting = computed(() => face.value?.kind === 'requesting' && !face.value.incoming)
  const since = ref(Date.now())
  watch(
    () => [requesting.value, scope()] as const,
    ([isRequesting, currentScope], [wasRequesting, previousScope]) => {
      if (isRequesting && (!wasRequesting || currentScope !== previousScope)) {
        since.value = Date.now()
      }
    },
  )

  const slot = computed<ActivitySlotState | null>(() =>
    face.value ? { ...face.value, since: since.value } : null,
  )

  return { heldId, slot }
}

/** The id of a message on its way; a failed delivery is not on its way. */
function deliveringId(submission: PendingSubmissionState | null): string | null {
  return submission && submission.error === null ? submission.id : null
}
