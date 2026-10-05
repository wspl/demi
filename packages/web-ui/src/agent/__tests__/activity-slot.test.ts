import { expect, spyOn, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { Block, SessionPhase, ToolCallStatus } from '@demicodes/protocol'
import type { MessageListBlock } from '../pending-steers'
import { activitySlotKind, type PendingAction } from '../activity-slot'
import { listTailBlocks } from '../list-tail'
import type { SessionLoad } from '../session-status'
import type { PendingSubmissionState } from '../types'
import { useActivitySlot } from '../useActivitySlot'

function kind(
  phase: SessionPhase,
  transcriptBlocks: readonly MessageListBlock[],
  renderBlocks: readonly MessageListBlock[] = transcriptBlocks,
  options: { load?: SessionLoad; pendingAction?: PendingAction } = {},
) {
  return activitySlotKind({
    load: options.load ?? 'ready',
    phase,
    pendingAction: options.pendingAction ?? null,
    startingTurn: false,
    transcriptBlocks,
    renderBlocks,
  })
}

test('a running turn requests while it waits for the first model output', () => {
  expect(kind('running', [])).toBe('requesting')
})

test('a running turn requests after user-like blocks', () => {
  expect(kind('running', [userBlock()])).toBe('requesting')
  expect(kind('running', [steerBlock()])).toBe('requesting')
  expect(kind('running', [pendingSteerBlock()])).toBe('requesting')
  expect(kind('running', [compactionMarkerBlock()])).toBe('requesting')
})

test('a running turn requests after a completed tool while waiting for the model to continue', () => {
  expect(kind('running', [toolCallBlock('completed')])).toBe('requesting')
  expect(kind('running', [toolCallBlock('error')])).toBe('requesting')
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

test('a pending steer does not add the slot while active thinking is already the tail', () => {
  const transcriptBlocks = [thinkingBlock()]
  const renderBlocks = [...transcriptBlocks, pendingSteerBlock()]
  expect(kind('running', transcriptBlocks, renderBlocks)).toBeNull()
})

test('a materialized steer requests again before the model continues', () => {
  expect(kind('running', [thinkingBlock(), steerBlock()])).toBe('requesting')
})

test('a queued tail neither hides nor adds the slot', () => {
  const waiting = [userBlock()]
  expect(kind('running', waiting, [...waiting, ...queuedTail()])).toBe('requesting')
  const thinking = [thinkingBlock()]
  expect(kind('running', thinking, [...thinking, ...queuedTail()])).toBeNull()
})

test('a recovery in flight requests: the hidden record is the transcript tail, the list ends before it', () => {
  const transcript = [userBlock(), errorBlock()]
  expect(kind('running', transcript, [userBlock()])).toBe('requesting')
  expect(kind('running', [userBlock(), abortBlock()])).toBe('requesting')
})

test('a pending resume says Requesting, after an error record and after a Stop alike', () => {
  expect(kind('idle', [userBlock(), errorBlock()], [userBlock()], { pendingAction: 'resume' })).toBe('requesting')
  expect(kind('idle', [userBlock(), abortBlock()], undefined, { pendingAction: 'resume' })).toBe('requesting')
})

test('connecting wins over a pending recovery and a running turn', () => {
  expect(kind('idle', [], undefined, { load: 'reconnecting' })).toBe('connecting')
  expect(kind('idle', [errorBlock()], [], { load: 'reconnecting', pendingAction: 'resume' })).toBe('connecting')
  expect(kind('running', [thinkingBlock()], undefined, { load: 'reconnecting' })).toBe('connecting')
})

/**
 * The tail row as the message list drives it, over a conversation the test
 * changes the way the page's state changes, at a clock the test sets.
 */
function slotOver(phase: SessionPhase, blocks: MessageListBlock[]) {
  const clock = spyOn(Date, 'now').mockReturnValue(0)
  const conversation = reactive({
    phase,
    blocks,
    pendingSubmission: null as PendingSubmissionState | null,
  })
  const scope = effectScope()
  const { slot } = scope.run(() => useActivitySlot({
    input: () => ({
      load: 'ready',
      phase: conversation.phase,
      pendingAction: null,
      transcriptBlocks: conversation.blocks,
      renderBlocks: [
        ...conversation.blocks,
        ...listTailBlocks({
          phase: conversation.phase,
          pendingSteers: [],
          queue: [],
          pendingSubmission: conversation.pendingSubmission,
        }),
      ],
    }),
    pendingSubmission: () => conversation.pendingSubmission,
    tail: () => conversation.blocks.at(-1),
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
  return { id: 'turn-1', text: 'hello', attachments: [], error }
}

test('a message sent to an idle conversation says Requesting from the send, through its confirmation, on one clock', async () => {
  using session = slotOver('idle', [textBlock()])
  const { conversation, slot, at } = session
  expect(slot.value).toBeNull()

  at(5_000)
  conversation.pendingSubmission = sent()
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', incoming: null, since: 5_000 })

  // The backend starts the turn before it writes the message.
  at(6_000)
  conversation.phase = 'running'
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', incoming: null, since: 5_000 })

  // The message's block confirms it; the page drops the pending message in the same update.
  at(8_000)
  conversation.blocks = [textBlock(), userBlock()]
  conversation.pendingSubmission = null
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', incoming: null, since: 5_000 })
})

test('a failed delivery ends the wait, and Retry starts it again from the retry', async () => {
  using session = slotOver('idle', [])
  const { conversation, slot, at } = session
  conversation.pendingSubmission = sent()
  await nextTick()
  conversation.pendingSubmission = sent('Connection closed before confirmation')
  await nextTick()
  expect(slot.value).toBeNull()

  at(9_000)
  conversation.pendingSubmission = sent()
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', incoming: null, since: 9_000 })
})

test('a message sent during a turn joins the queue and adds no Requesting under the streaming answer', async () => {
  using session = slotOver('running', [userBlock(), textBlock()])
  const { conversation, slot } = session
  conversation.pendingSubmission = sent()
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

function pendingSteerBlock(): MessageListBlock {
  return {
    type: 'pending_steer',
    id: 'pending-steer-1',
    pendingSteerId: 'pending-1',
    content: [{ type: 'text', text: 'steer' }],
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
    createdAt,
    model,
    toolUseId: `tool-use-${status}`,
    toolName: 'shell_exec',
    input: '{"script":"ls"}',
    status,
    output: [],
    view: null,
  }
}

function queuedTail(): MessageListBlock[] {
  return [
    { type: 'queue_divider', id: 'queue-divider', count: 1 },
    {
      type: 'queued_message',
      id: 'queued:q1',
      queueId: 'q1',
      content: [{ type: 'text', text: 'follow-up' }],
    },
  ]
}

function thinkingBlock(): MessageListBlock {
  return {
    type: 'thinking',
    id: 'thinking-1',
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
