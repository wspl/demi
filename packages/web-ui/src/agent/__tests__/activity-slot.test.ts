import { expect, test } from 'bun:test'
import type { SessionPhase, ToolCallStatus } from '@demicodes/core'
import type { DisplayedBlock as Block } from '@demicodes/agent/client'
import type { MessageListBlock } from '../pending-steers'
import { activitySlotKind, type PendingAction } from '../activity-slot'
import type { SessionLoad } from '../session-status'

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
  expect(kind('running', [compactionBoundaryBlock()])).toBe('requesting')
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

test('a pending resume says Retrying after an error record and Resuming after an abort', () => {
  expect(kind('idle', [userBlock(), errorBlock()], [userBlock()], { pendingAction: 'resume' })).toBe('retrying')
  expect(kind('idle', [userBlock(), abortBlock()], undefined, { pendingAction: 'resume' })).toBe('resuming')
})

test('connecting wins over a pending recovery and a running turn', () => {
  expect(kind('idle', [], undefined, { load: 'reconnecting' })).toBe('connecting')
  expect(kind('idle', [errorBlock()], [], { load: 'reconnecting', pendingAction: 'resume' })).toBe('connecting')
  expect(kind('running', [thinkingBlock()], undefined, { load: 'reconnecting' })).toBe('connecting')
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

function compactionBoundaryBlock(): MessageListBlock {
  return {
    type: 'compaction_boundary',
    id: 'compaction-1',
    createdAt,
    model,
    summary: 'summary',
    summaryTokens: 1,
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
    streamingOutput: [],
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
