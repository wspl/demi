import { computed, ref, watch, type ComputedRef } from 'vue'
import type { Block, PendingCall, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { assistantFooterIds, replyEndIds } from './assistant-footer'
import { isHandoffBlock, type PendingAction } from './activity-slot'
import { listTailBlocks } from './list-tail'
import type { ActivitySlotRow, MessageListBlock } from './pending-steers'
import type { SessionLoad } from './session-status'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import { useActivitySlot, type ActivitySlotState } from './useActivitySlot'
import { getVisibleBlocks } from './visible-blocks'
import { groupWork, workRowId } from './work-groups'

/** What a transcript list shows its rows from, as `AgentMessageList` takes it. */
export interface MessageRowsSource {
  conversationId: string
  blocks: readonly Block[]
  pendingCalls?: readonly PendingCall[]
  /** When the agent began to wait for the model's answer, as the server tells it; null when it waits for none. */
  waitingSince?: number | null
  pendingSteers: readonly PendingSteerMessage[]
  queue: readonly QueuedMessage[]
  phase: SessionPhase
  load?: SessionLoad
  backendAway?: boolean
  pendingAction?: PendingAction
  pendingSubmission?: PendingSubmissionState | null
  atStart: boolean
  atEnd: boolean
  latest: boolean
}

export interface MessageRows {
  /** The transcript's blocks the list shows any of. */
  visibleBlocks: ComputedRef<Block[]>
  /** The transcript's blocks that are rows or steps of a row, without the one rolling into the tail row. */
  transcriptRows: ComputedRef<Block[]>
  /**
   * The list's rows, in order, the tail row among them right after the
   * turn's own rows and above the messages that wait; `rowKey` names each
   * across its forms.
   */
  rows: ComputedRef<MessageListBlock[]>
  /** The tail row: Requesting, Connecting, or a block rolling in; null when none shows. */
  slot: ComputedRef<ActivitySlotState | null>
  /** The texts that end the agent's reply, which Copy and Fork close. */
  footerIds: ComputedRef<Set<string>>
  /** The key of the row the block rolling into the tail row becomes, which the tail row already carried into place. */
  heldKey: ComputedRef<string | null>
}

/**
 * The rows a transcript list shows and what closes them, derived from the
 * conversation's state (`product.md` § Following the agent's work,
 * `runtime.md` § Work groups): the transcript's blocks with the run of steps
 * grouped, then the tail, the tail row under them, and the footers. A row is
 * named by `rowKey`, which it keeps as it changes form: the message being
 * sent and the message confirmed, a run from its first step to its end.
 * The run at the end of a running turn is one row whose `live` says the
 * turn has not moved past it, and its face shimmers for as long; no
 * Requesting row shows under it. The messages that wait, the steers and the
 * queue, stay below the turn: the tail row stands above them, and its kind
 * is read from the transcript alone.
 */
export function useMessageRows(source: () => MessageRowsSource): MessageRows {
  const visibleBlocks = computed(() => getVisibleBlocks(source().blocks))
  // A recovery hides the record it recovers from: the tail row names the recovery, then the turn running.
  const transcriptBlocks = computed(() => {
    const { phase, pendingAction } = source()
    const visible = visibleBlocks.value
    const recovering = phase !== 'idle' || pendingAction === 'resume'
    return recovering && visible.at(-1)?.type === 'error' ? visible.slice(0, -1) : visible
  })
  const tail = computed(() => {
    const input = source()
    return !input.latest ? { turn: [], waiting: [] } : listTailBlocks({
      phase: input.phase,
      blocks: input.blocks,
      pendingCalls: input.pendingCalls ?? [],
      pendingSteers: input.pendingSteers,
      queue: input.queue,
      pendingSubmission: input.pendingSubmission,
    })
  })
  // The calls the model is writing are the newest steps of the work: they join
  // its group and, as any step, roll into the tail row first.
  const pendingCallRows = computed(() => tail.value.turn.filter((block) => block.type === 'pending_call'))
  const otherTurnBlocks = computed(() => tail.value.turn.filter((block) => block.type !== 'pending_call'))
  const workBlocks = computed<MessageListBlock[]>(() => [...transcriptBlocks.value, ...pendingCallRows.value])
  const running = computed(() => source().phase === 'running')

  const { heldId, slot, startingTurn } = useActivitySlot({
    input: () => {
      const input = source()
      return {
        load: input.load ?? 'ready',
        backendAway: input.backendAway ?? false,
        phase: input.phase,
        pendingAction: input.pendingAction ?? null,
        transcriptBlocks: visibleBlocks.value,
        turnRows: groupWork(workBlocks.value, running.value),
      }
    },
    pendingSubmission: () => source().pendingSubmission ?? null,
    waitingSince: () => source().waitingSince ?? null,
    tail: () => workBlocks.value.at(-1),
    scope: () => source().conversationId,
  })

  /** The block rolling into the tail row, which is not a list row yet. */
  const held = computed(() => {
    const last = workBlocks.value.at(-1)
    return heldId.value !== null && last?.id === heldId.value ? last : null
  })
  const transcriptRows = computed(() => held.value ? transcriptBlocks.value.filter((block) => block !== held.value) : transcriptBlocks.value)
  const workRows = computed(() => groupWork(
    held.value ? workBlocks.value.slice(0, -1) : workBlocks.value,
    running.value,
  ))
  // Each wait the tail row shows is counted, so a new one enters as a row
  // does, once, and a virtual row remounted by scrolling stays still.
  const waits = ref(0)
  watch(() => slot.value !== null, (shown) => {
    if (shown) {
      waits.value += 1
    }
  }, { immediate: true, flush: 'sync' })
  // Only the latest window shows the turn's tail rows.
  const slotRow = computed<ActivitySlotRow[]>(() => slot.value && source().latest
    ? [{ type: 'activity_slot', id: `tail-row:${waits.value}`, slot: slot.value }]
    : [])
  const rows = computed<MessageListBlock[]>(() => [
    ...(source().atStart ? [] : [{ type: 'history_edge', id: 'history-before' } as const]),
    ...workRows.value,
    ...otherTurnBlocks.value,
    ...slotRow.value,
    ...tail.value.waiting,
    ...(source().atEnd ? [] : [{ type: 'history_edge', id: 'history-after' } as const]),
  ])

  // A message on its way that starts a turn is the user's next message
  // already, so the answer before it keeps its footer while the turn starts
  // and the message is written.
  const footerIds = computed(() => assistantFooterIds(
    visibleBlocks.value,
    replyEndIds(visibleBlocks.value, source().phase, startingTurn.value),
  ))
  const heldKey = computed(() => held.value && isHandoffBlock(held.value) ? workRowId(held.value) : null)

  return { visibleBlocks, transcriptRows, rows, slot, footerIds, heldKey }
}

