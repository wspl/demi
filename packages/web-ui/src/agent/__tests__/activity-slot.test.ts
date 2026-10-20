import { expect, spyOn, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { Block, SessionPhase, ToolCallStatus } from '@demicodes/protocol'
import type { MessageListBlock } from '../pending-steers'
import { activitySlotKind } from '../activity-slot'
import { groupWork } from '../work-groups'
import type { SessionLoad } from '../session-status'
import { turnStartingMessage } from '../send-way'
import type { PendingSubmissionState } from '../types'
import { useActivitySlot } from '../useActivitySlot'

function kind(
  phase: SessionPhase,
  transcriptBlocks: readonly MessageListBlock[],
  turnRows: readonly MessageListBlock[] = transcriptBlocks,
  options: { load?: SessionLoad; backendAway?: boolean; recovering?: boolean } = {},
) {
  return activitySlotKind({
    load: options.load ?? 'ready',
    backendAway: options.backendAway ?? false,
    phase,
    recovering: options.recovering ?? false,
    startingTurn: false,
    transcriptBlocks,
    turnRows,
  })
}

test('a first opening whose socket is not made yet says it connects, as a lost socket does', () => {
  expect(kind('idle', [userBlock()], undefined, { load: 'connecting' })).toBe('connecting')
  expect(kind('idle', [userBlock()], undefined, { load: 'connecting', backendAway: true })).toBeNull()
  expect(kind('idle', [userBlock()], undefined, { load: 'opening' })).toBeNull()
})

test('a running turn requests while it waits for the first model output', () => {
  expect(kind('running', [])).toBe('requesting')
})

test('a running turn requests after user-like blocks', () => {
  expect(kind('running', [userBlock()])).toBe('requesting')
  expect(kind('running', [steerBlock()])).toBe('requesting')
  expect(kind('running', [compactionMarkerBlock()])).toBe('requesting')
})

test('a running turn requests after a completed tool while waiting for the model to continue', () => {
  expect(kind('running', [toolCallBlock('completed')])).toBe('requesting')
  expect(kind('running', [toolCallBlock('error')])).toBe('requesting')
})

// A run's row holds only the steps that ended, so at a running turn's end it
// is what has happened, and the turn waits for the model under it; a step that
// still runs is a row of its own after the run, the turn's activity, and no
// Requesting row shows while it runs. Before, the run's row stood for the
// wait, and no Requesting showed under it.
test('a run whose steps ended waits for the model under it; a step that runs is the activity', () => {
  const transcript = [userBlock(), thinkingBlock(), toolCallBlock('completed')]
  expect(kind('running', transcript, groupWork(transcript, true))).toBe('requesting')
  const lone = [userBlock(), toolCallBlock('completed')]
  expect(kind('running', lone, groupWork(lone, true))).toBe('requesting')
  const runs = [userBlock(), toolCallBlock('executing')]
  expect(kind('running', runs, groupWork(runs, true))).toBeNull()
})

// Review: a call still waiting for its result keeps the turn waiting for it,
// wherever it stands: the agent started A, wrote a line, and B in a new run
// returned. Before, the run of B at the tail read as the model's turn.
test('no Requesting shows while a call in an earlier run still executes', () => {
  const started = { ...toolCallBlock('executing'), run: 'thinking-1' } as MessageListBlock
  const said = { ...textBlock(), forkable: true } as MessageListBlock
  const returned = { ...toolCallBlock('completed'), id: 'tool-b', toolUseId: 'tool-use-b', run: 'tool-b' } as MessageListBlock
  const transcript = [userBlock(), { ...thinkingBlock(), run: 'thinking-1' } as MessageListBlock, started, said, returned]
  expect(kind('running', transcript, groupWork(transcript, true))).toBeNull()
})

test('the slot stays hidden while the tool row itself is executing', () => {
  expect(kind('running', [toolCallBlock('executing')])).toBeNull()
})

test('the slot stays hidden outside running phase', () => {
  expect(kind('idle', [])).toBeNull()
  expect(kind('idle', [toolCallBlock('completed')])).toBeNull()
  expect(kind('compacting', [userBlock()])).toBeNull()
})

test('the slot stays hidden while assistant content is the latest visible block', () => {
  expect(kind('running', [thinkingBlock()])).toBeNull()
  expect(kind('running', [textBlock()])).toBeNull()
})

test('finished text with nothing after it is followed by Requesting, and a call being written takes its place', () => {
  const finished: MessageListBlock = { ...textBlock(), forkable: true } as MessageListBlock
  expect(kind('running', [finished])).toBe('requesting')
  const writing: MessageListBlock = {
    type: 'pending_call',
    id: 'pending-call:call-1',
    call: { toolUseId: 'call-1', toolName: 'shell', description: 'Write the categorizer' },
  }
  expect(kind('running', [finished], [finished, writing])).toBeNull()
  expect(kind('idle', [finished])).toBeNull()
})

test('a materialized steer requests again before the model continues', () => {
  expect(kind('running', [thinkingBlock(), steerBlock()])).toBe('requesting')
})

// A Stop being written, before the phase goes idle, waits for nothing.
test('a running turn does not request after a Stop being written', () => {
  expect(kind('running', [userBlock(), abortBlock()])).toBeNull()
})

test('a recovery this page asked for says Requesting, after an error record and after a Stop alike', () => {
  expect(kind('idle', [userBlock(), errorBlock()], [userBlock()], { recovering: true })).toBe('requesting')
  expect(kind('idle', [userBlock(), abortBlock()], undefined, { recovering: true })).toBe('requesting')
  expect(kind('running', [userBlock(), errorBlock()], [userBlock()], { recovering: true })).toBe('requesting')
})

test('a conversation that failed to open waits for nothing: the failure told at the tail says so', () => {
  expect(kind('running', [userBlock()], undefined, { load: 'failed' })).toBeNull()
})

test('connecting wins over a pending recovery and a running turn', () => {
  expect(kind('idle', [], undefined, { load: 'reconnecting' })).toBe('connecting')
  expect(kind('idle', [errorBlock()], [], { load: 'reconnecting', recovering: true })).toBe('connecting')
  expect(kind('running', [thinkingBlock()], undefined, { load: 'reconnecting' })).toBe('connecting')
})

test('while the connection banner says the backend is away, a reconnecting conversation shows no row of its own', () => {
  const away = { load: 'reconnecting', backendAway: true } as const
  expect(kind('idle', [], undefined, away)).toBeNull()
  expect(kind('running', [userBlock()], undefined, away)).toBeNull()
  expect(kind('idle', [errorBlock()], [], { ...away, recovering: true })).toBeNull()
  // Its socket lost while the page still reaches the backend: the row says so.
  expect(kind('running', [userBlock()], undefined, { load: 'reconnecting' })).toBe('connecting')
})

/**
 * The tail row as the message list drives it, over a conversation the test
 * changes the way the page's state changes, at a clock the test sets.
 */
function slotOver(phase: SessionPhase, blocks: MessageListBlock[], opened = 0) {
  const clock = spyOn(Date, 'now').mockReturnValue(opened)
  const conversation = reactive({
    phase,
    blocks,
    load: 'ready' as SessionLoad,
    pendingSubmissions: [] as PendingSubmissionState[],
    waitingSince: null as number | null,
  })
  const scope = effectScope()
  const { slot } = scope.run(() => useActivitySlot({
    input: () => ({
      load: conversation.load,
      backendAway: false,
      phase: conversation.phase,
      recovering: false,
      transcriptBlocks: conversation.blocks,
      turnRows: conversation.blocks,
    }),
    pendingSubmissions: () => conversation.pendingSubmissions,
    waitingSince: () => conversation.waitingSince,
    scope: () => 'conversation',
  }))!
  return {
    conversation,
    slot,
    at: (ms: number) => clock.mockReturnValue(ms),
    [Symbol.dispose]() {
      scope.stop()
      clock.mockRestore()
    },
  }
}

function sent(error: string | null = null): PendingSubmissionState {
  return { id: 'turn-1', way: 'send', text: 'hello', attachments: [], error }
}

test('a message sent to an idle conversation says Requesting from the send, through its confirmation, on one clock', async () => {
  using session = slotOver('idle', [textBlock()])
  const { conversation, slot, at } = session
  expect(slot.value).toBeNull()

  at(5_000)
  conversation.pendingSubmissions = [sent()]
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })

  // The backend starts the turn before it writes the message.
  at(6_000)
  conversation.phase = 'running'
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })

  // The message's block confirms it; the page drops the pending message in the same update.
  at(8_000)
  conversation.blocks = [textBlock(), userBlock()]
  conversation.pendingSubmissions = []
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })
})

test('a view shown during a wait counts from the start the server tells, not from when it opened', async () => {
  // A subagent's request had waited eight minutes when its view opened.
  using session = slotOver('running', [userBlock()], 600_000)
  const { conversation, slot, at } = session
  conversation.waitingSince = 120_000
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 120_000 })

  at(660_000)
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 120_000 })
})

test('a message this page sent counts from its send, also once the server tells a later start', async () => {
  using session = slotOver('idle', [textBlock()])
  const { conversation, slot, at } = session
  at(5_000)
  conversation.pendingSubmissions = [sent()]
  await nextTick()
  at(6_000)
  conversation.phase = 'running'
  conversation.waitingSince = 5_400
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })

  // The answer streams, then its call ran: the wait for the model's next
  // answer is the server's, from when the call's result went back.
  at(9_000)
  conversation.blocks = [textBlock(), userBlock(), textBlock()]
  conversation.pendingSubmissions = []
  conversation.waitingSince = null
  await nextTick()
  expect(slot.value).toBeNull()
  at(20_100)
  conversation.blocks = [...conversation.blocks, toolCallBlock('completed')]
  conversation.waitingSince = 20_000
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 20_000 })
})

// The request has shown nothing for 74 s when the user sends a steer now:
// the server starts the wait again, and the sending page counts from there
// as every other page does.
test('a message this page sent counts from its send only for its own wait, not for one a steer sent now starts', async () => {
  using session = slotOver('idle', [textBlock()])
  const { conversation, slot, at } = session
  at(5_000)
  conversation.pendingSubmissions = [sent()]
  await nextTick()
  at(6_000)
  conversation.phase = 'running'
  conversation.waitingSince = 5_400
  conversation.blocks = [textBlock(), userBlock()]
  conversation.pendingSubmissions = []
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })

  at(79_400)
  conversation.waitingSince = 79_400
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 79_400 })
})

test('a failed delivery ends the wait, and Retry starts it again from the retry', async () => {
  using session = slotOver('idle', [])
  const { conversation, slot, at } = session
  conversation.pendingSubmissions = [sent()]
  await nextTick()
  conversation.pendingSubmissions = [sent('Connection closed before confirmation')]
  await nextTick()
  expect(slot.value).toBeNull()

  at(9_000)
  conversation.pendingSubmissions = [sent()]
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 9_000 })
})

test('a message sent while the socket reconnects says Connecting, and Requesting starts once the socket is back', async () => {
  using session = slotOver('idle', [])
  const { conversation, slot, at } = session
  conversation.load = 'reconnecting'
  conversation.pendingSubmissions = [sent()]
  await nextTick()
  expect(slot.value?.kind).toBe('connecting')

  at(12_000)
  conversation.load = 'ready'
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 12_000 })
})

// Back-to-back sends 400 ms apart, the first one failing: the composer's
// Stop and the transcript's Requesting read one rule, so the one shows
// exactly while the other does. Before, the composer kept Stop for the
// second message, a steer that goes as the next message, while the row
// showed nothing. About 1 ms.
test('the composer shows Stop exactly while the row says Requesting for a message that starts a turn', async () => {
  using session = slotOver('idle', [textBlock()])
  const { conversation, slot } = session
  // As ConversationComposer passes it: running, or a message on its way
  // that starts a turn; SessionComposer shows Stop while running or compacting.
  const stopShown = () =>
    conversation.phase !== 'idle'
    || turnStartingMessage(conversation.phase, conversation.pendingSubmissions) !== undefined
  const shown = async () => {
    await nextTick()
    return { stop: stopShown(), requesting: slot.value?.kind === 'requesting' }
  }
  const message = (id: string, way: PendingSubmissionState['way'], error: string | null = null): PendingSubmissionState =>
    ({ id, way, text: id, attachments: [], error })
  const both = (shownNow: boolean) => ({ stop: shownNow, requesting: shownNow })

  expect(await shown()).toEqual(both(false))
  conversation.pendingSubmissions = [message('a', 'send')]
  expect(await shown()).toEqual(both(true))
  // Sent while the first is on its way: it steers the turn the first starts.
  conversation.pendingSubmissions = [message('a', 'send'), message('b', 'steer')]
  expect(await shown()).toEqual(both(true))
  // The first is not delivered: the session answers the steer as the next message.
  conversation.pendingSubmissions = [message('a', 'send', 'Upload failed'), message('b', 'steer')]
  expect(await shown()).toEqual(both(true))
  conversation.pendingSubmissions = [message('a', 'send', 'Upload failed'), message('b', 'steer', 'Upload failed')]
  expect(await shown()).toEqual(both(false))
  // Retry sends the first again as the next message.
  conversation.pendingSubmissions = [message('a', 'send'), message('b', 'steer', 'Upload failed')]
  expect(await shown()).toEqual(both(true))
  // The backend starts its turn before it writes the message.
  conversation.phase = 'running'
  expect(await shown()).toEqual(both(true))
})

test('a message sent during a turn joins the queue and adds no Requesting under the streaming answer', async () => {
  using session = slotOver('running', [userBlock(), textBlock()])
  const { conversation, slot } = session
  conversation.pendingSubmissions = [{ ...sent(), way: 'queue' }]
  await nextTick()
  expect(slot.value).toBeNull()
})

const createdAt = '2026-06-24T00:00:00.000Z'
const model = null as unknown as BlockWithModel['model']

type BlockWithModel = Extract<Block, { model: unknown }>

function userBlock(): MessageListBlock {
  return {
    type: 'user',
    id: 'user-1',
    turnId: 'turn-1',
    createdAt,
    model,
    content: [{ type: 'text', text: 'hello' }],
    preamble: null,
  }
}

function steerBlock(): MessageListBlock {
  return {
    type: 'steer',
    id: 'steer-1',
    turnId: 'turn-1',
    createdAt,
    model,
    content: [{ type: 'text', text: 'steer' }],
  }
}

/** A compaction inside a turn shows as its marker at the tail; the turn then asks the model again. */
function compactionMarkerBlock(): MessageListBlock {
  return {
    type: 'compaction_marker',
    id: 'compaction-marker-1',
    createdAt,
    model,
    boundaryId: 'compaction-1',
    compactedTokens: 1,
  }
}

function toolCallBlock(status: ToolCallStatus): MessageListBlock {
  return {
    type: 'tool_call',
    id: `tool-${status}`,
    run: `tool-${status}`,
    createdAt,
    model,
    toolUseId: `tool-use-${status}`,
    toolName: 'shell',
    input: '{"script":"ls"}',
    status,
    endedAt: status === 'executing' ? null : createdAt, output: [],
    view: null,
  }
}

function thinkingBlock(): MessageListBlock {
  return {
    type: 'thinking',
    id: 'thinking-1',
    run: 'thinking-1',
    createdAt,
    model,
    text: 'thinking',
    signature: null,
  }
}

function errorBlock(): MessageListBlock {
  return {
    type: 'error',
    id: 'error-1',
    createdAt,
    model,
    message: 'Overloaded',
    code: 'overloaded',
  }
}

function abortBlock(): MessageListBlock {
  return {
    type: 'abort',
    id: 'abort-1',
    createdAt,
    model,
    isResumed: false,
  }
}

function textBlock(): MessageListBlock {
  return {
    type: 'text',
    id: 'text-1',
    createdAt,
    model,
    text: 'answer',
  }
}
