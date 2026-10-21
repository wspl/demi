import { computed, ref, toRaw, watch, type ComputedRef } from 'vue'
import type { Block, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { assistantFooterIds, replyEndIds } from './assistant-footer'
import type { PendingAction } from './activity-slot'
import { listTailBlocks, submissionRows, undeliveredPlaces } from './list-tail'
import { sentMessage } from './message-delivery'
import type { ActivitySlotRow, MessageListBlock, StepsNotSharedRow } from './pending-steers'
import { rowKey, succeedKeys, type DrawnRow } from './row-key'
import type { SessionFailureNotice, SessionLoad } from './session-status'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import { AT_REST, type AgentActivity } from './agent-activity'
import { useActivitySlot, type ActivitySlotState } from './useActivitySlot'
import { getVisibleBlocks } from './visible-blocks'
import { workGrouping } from './work-groups'
import type { LiveCallLookup } from './live-calls'
import { shellRowRunning } from './terminals'

/**
 * What a transcript list shows its rows from, as `AgentMessageList` takes it.
 */
export interface MessageRowsSource {
  conversationId: string
  blocks: readonly Block[]
  /** What the agent does now; absent for one at rest, as a share's. */
  activity?: AgentActivity
  pendingSteers: readonly PendingSteerMessage[]
  queue: readonly QueuedMessage[]
  /** The message the running turn started with, until the turn writes it. */
  starting?: QueuedMessage | null
  phase: SessionPhase
  load?: SessionLoad
  backendAway?: boolean
  pendingAction?: PendingAction
  /** A session-level failure told at the tail of the transcript. */
  failure?: SessionFailureNotice | null
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSubmissions?: readonly PendingSubmissionState[]
  /**
   * The waiting messages whose Recall is on its way, by id, which show nowhere.
   */
  removing?: readonly string[]
  atStart: boolean
  atEnd: boolean
  latest: boolean
  /**
   * The runs of steps a share does not hold, each after the row of the block it
   * follows.
   */
  stepsNotShared?: readonly StepsNotSharedRow[]
}

export interface MessageRows {
  /** The transcript's blocks the list shows any of. */
  visibleBlocks: ComputedRef<Block[]>
  /** The transcript's blocks that are rows or steps of a row. */
  transcriptRows: ComputedRef<Block[]>
  /**
   * The list's rows, in order, the tail row among them right after the
   * turn's own rows and above the messages that wait.
   */
  rows: ComputedRef<MessageListBlock[]>
  /**
   * The key the list draws a row of `rows` under: its own (`rowKey`), or
   * the key of the row it took the place of (`succeedKeys`).
   */
  keyOf: (row: MessageListBlock) => string
  /** The tail row: Requesting or Connecting; null when none shows. */
  slot: ComputedRef<ActivitySlotState | null>
  /** The texts that end the agent's reply, which Copy and Fork close. */
  footerIds: ComputedRef<Set<string>>
}

/**
 * A recovery this page asked for: the record it continues from, and the blocks
 * the transcript showed at the press.
 */
interface Recovery {
  record: string
  shown: ReadonlySet<string>
}

/**
 * The rows a transcript list shows and what closes them, derived from the
 * conversation's state: the transcript's blocks with the run of steps grouped,
 * then the tail, the tail row under them, and the footers.
 *
 * A row is named by `rowKey`, which it keeps as it changes form: the message
 * being sent and the message confirmed, a run from its first step to its end.
 * Whatever replaces a row of another name takes its key (`keyOf`): the step
 * that ends a wait, the Requesting row of a Continue, the answer that follows
 * thinking without text.
 *
 * A run's row holds the steps that ended. A step that still runs is a row of
 * its own under it, in the tail row's place, and once it ended the turn waits
 * for the model in a Requesting row under the run. A call whose command runs
 * on after it returned still runs, as the command's live frames in
 * `liveCalls` say, until the command ends.
 *
 * The messages that wait, the steers and the queue, stay below the turn: the
 * tail row stands above them. Whether it waits is the server's word or this
 * page's own action's (`useActivitySlot`), never the transcript's.
 *
 * Continue after a Stop and Resume after an error take the place of the record
 * they continue from. From the press, the record's row is the Requesting row,
 * until the turn shows what comes after it, or the request is refused and the
 * record shows again. A step that comes after it joins the run above, as the
 * backend stores it, and the Requesting row gives way: the one change that
 * shrinks the list, which the user asked for.
 */
export function useMessageRows(
  source: () => MessageRowsSource,
  liveCalls: LiveCallLookup = () => undefined,
): MessageRows {
  // The transcript as plain data: a block that changes is replaced, as is the
  // list that holds it (`applyTranscriptPatches`), so a new list is the one
  // change to follow. Reading each block through a reactive proxy would track
  // every field of a long transcript at every change. A list made from a
  // reactive one holds proxies of the blocks it kept, so each block is
  // unwrapped too, which keeps an unchanged block the same object.
  const blocks = computed(() => toRaw(source().blocks).map(toRaw))
  const visibleBlocks = computed(() => getVisibleBlocks(blocks.value))

  // A recovery this page asked for lasts from the press until the session
  // is idle again with nothing pending: the turn it ran ended, or the server
  // refused it, and the record shows again.
  const recovery = ref<Recovery | null>(null)
  watch(
    [() => source().pendingAction, () => source().phase, () => source().conversationId],
    ([action, phase, scope], previous) => {
      if (previous && scope !== previous[2]) {
        recovery.value = null
      }
      if (action === 'resume') {
        recovery.value ??= recoveryFrom(visibleBlocks.value)
      } else if (phase === 'idle') {
        recovery.value = null
      }
    },
    { immediate: true },
  )
  // Where each message not delivered stands: before the first message sent
  // after it that shows, by the messages each row shows.
  const undelivered = computed(() => {
    const input = source()
    const shown = new Set(blocks.value.flatMap((block) =>
      block.type === 'user' ? [block.turnId] : block.type === 'steer' ? [block.id] : []))
    if (input.latest) {
      for (const message of [...input.pendingSteers, ...input.queue, ...(input.starting ? [input.starting] : [])]) {
        shown.add(message.id)
      }
      for (const submission of input.pendingSubmissions ?? []) {
        if (submission.error === null) {
          shown.add(submission.id)
        }
      }
    }
    return undeliveredPlaces({
      submissions: input.pendingSubmissions ?? [],
      shown,
      removing: new Set(input.removing),
      latest: input.latest,
      atStart: input.atStart,
    })
  })
  const tail = computed(() => {
    const input = source()
    return !input.latest ? { turn: [], waiting: [] } : listTailBlocks({
      phase: input.phase,
      blocks: blocks.value,
      pendingCalls: input.activity?.pendingCalls ?? [],
      pendingSteers: input.pendingSteers,
      queue: input.queue,
      starting: input.starting,
      pendingSubmissions: input.pendingSubmissions ?? [],
      removing: input.removing,
      elsewhere: new Set([...undelivered.value].flatMap(([id, place]) => place === 'end' ? [] : [id])),
    })
  })
  const { slot, startingTurn } = useActivitySlot({
    input: () => {
      const input = source()
      return {
        load: input.load ?? 'ready',
        backendAway: input.backendAway ?? false,
      }
    },
    phase: () => source().phase,
    pendingSubmissions: () => source().pendingSubmissions ?? [],
    activity: () => source().activity ?? AT_REST,
  })
  // The calls the model is writing are the newest steps of the work: each is a
  // row of its own after its run.
  const pendingCallRows = computed(() => tail.value.turn.filter((block) => block.type === 'pending_call'))
  const otherTurnBlocks = computed(() => tail.value.turn.filter((block) => block.type !== 'pending_call'))
  // Until the turn shows something it did not show at the press, the
  // record's place is the turn's tail, and the tail row stands there: the
  // record marked resumed or unwound, with the leftovers before it, leaves
  // only what was shown before. A call being written shows only from its
  // description, so only such a call is something new. With no tail row,
  // as while the server tells no wait yet, the record keeps its place.
  const atCut = computed(() => {
    const current = recovery.value
    if (current === null || slot.value === null || pendingCallRows.value.length > 0) {
      return false
    }
    const end = visibleBlocks.value.at(-1)
    return end === undefined || current.shown.has(end.id)
  })
  const hiddenId = computed(() => atCut.value ? recovery.value?.record ?? null : null)
  const transcriptRows = computed(() => visibleBlocks.value.filter((block) => block.id !== hiddenId.value))
  // The calls that returned while their command runs on. A command's output
  // changes its record many times a second, so the set stays the same object
  // until a command starts or stops running, and only then regroups.
  const runningOn = computed<ReadonlySet<string>>((previous) => {
    const calls = new Set(visibleBlocks.value.flatMap((block) =>
      block.type === 'tool_call' && block.status !== 'executing' && shellRowRunning(block.status, liveCalls(block.toolUseId))
        ? [block.toolUseId]
        : []))
    return previous !== undefined && previous.size === calls.size && [...calls].every((call) => previous.has(call))
      ? previous
      : calls
  })
  // Each run keeps its rows while it stays as it was, so a turn that
  // streams regroups only its own last run.
  const groupWork = workGrouping()
  const turnRows = computed(() => {
    const commandRuns = runningOn.value
    return groupWork(
      [...visibleBlocks.value, ...pendingCallRows.value],
      source().phase === 'running' && !atCut.value,
      (toolUseId) => commandRuns.has(toolUseId),
    )
  })

  // Each wait the tail row shows is counted, so a new one is a row of its own.
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
  // A share's runs of steps not shared, each after the row it follows.
  const sharedRows = computed<MessageListBlock[]>(() => {
    const rows = turnRows.value.filter((row) => row.id !== hiddenId.value)
    const steps = source().stepsNotShared ?? []
    if (steps.length === 0) {
      return rows
    }
    const after = (id: string | null) => steps.filter((row) => row.after === id)
    return [...after(null), ...rows.flatMap((row) => [row, ...after(row.id)])]
  })
  // The messages not delivered that stand before a message sent after them,
  // with the rows that tell their failures, by that message's id, in the
  // order they were sent.
  const placed = computed(() => {
    const before = new Map<string, MessageListBlock[]>()
    for (const submission of source().pendingSubmissions ?? []) {
      const place = undelivered.value.get(submission.id)
      if (place !== undefined && typeof place === 'object') {
        before.set(place.before, [...before.get(place.before) ?? [], ...submissionRows(submission)])
      }
    }
    return before
  })
  const rows = computed<MessageListBlock[]>(() => {
    const failure = source().failure
    const tailRows: MessageListBlock[] = [
      ...otherTurnBlocks.value,
      ...slotRow.value,
      ...(failure ? [{ type: 'session_failure', id: 'session-failure', failure } as const] : []),
      ...tail.value.waiting,
    ]
    const before = placed.value
    const inPlace = (list: readonly MessageListBlock[]) => before.size === 0 ? list : list.flatMap((row) => {
      const sent = sentMessage(row)?.id
      return sent === undefined ? [row] : [...before.get(sent) ?? [], row]
    })
    // One whose message shows in no row after all stands at the end of the
    // transcript.
    const present = new Set([...sharedRows.value, ...tailRows].flatMap((row) => sentMessage(row)?.id ?? []))
    const unplaced = [...before].flatMap(([sent, list]) => present.has(sent) ? [] : list)
    return [
      ...(source().atStart ? [] : [{ type: 'history_edge', id: 'history-before' } as const]),
      ...inPlace(sharedRows.value),
      ...unplaced,
      ...inPlace(tailRows),
      ...(source().atEnd ? [] : [{ type: 'history_edge', id: 'history-after' } as const]),
    ]
  })

  // The keys the list last drew, which the next rows succeed. Each list of
  // rows is keyed once, against the list keyed before it: a list no frame
  // drew in between is one the reader never saw either.
  let drawn: DrawnRow[] = []
  let drawnScope: string | null = null
  const keys = computed(() => {
    const scope = source().conversationId
    if (scope !== drawnScope) {
      drawn = []
      drawnScope = scope
    }
    const next = rows.value.map((row) => ({ natural: rowKey(row), fixed: row.type === 'history_edge' }))
    const assigned = succeedKeys(drawn, next)
    drawn = next.map((row, index) => ({ ...row, drawn: assigned[index]! }))
    return new Map(rows.value.map((row, index) => [row, assigned[index]!]))
  })
  const keyOf = (row: MessageListBlock): string => keys.value.get(row) ?? rowKey(row)

  // A message on its way that starts a turn is the user's next message
  // already, so the answer before it keeps its footer while the turn starts
  // and the message is written.
  const footerIds = computed(() => assistantFooterIds(
    visibleBlocks.value,
    replyEndIds(visibleBlocks.value, source().phase, startingTurn.value),
  ))

  return { visibleBlocks, transcriptRows, rows, keyOf, slot, footerIds }
}

/**
 * What a recovery continues from: the record that ends the visible
 * transcript, a Stop or an error, and the blocks shown with it; null when
 * the transcript ends otherwise.
 */
function recoveryFrom(blocks: readonly Block[]): Recovery | null {
  const end = blocks.at(-1)
  if (end?.type !== 'error' && end?.type !== 'abort') {
    return null
  }
  return { record: end.id, shown: new Set(blocks.map((block) => block.id)) }
}
