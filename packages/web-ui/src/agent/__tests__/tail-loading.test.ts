import { expect, test } from 'bun:test'
import type { Block, ToolCallStatus } from '@demi/core'
import type { MessageListBlock } from '../pending-steers'
import { shouldShowTailLoading } from '../tail-loading'

test('tail loading appears while a running turn waits for the first model output', () => {
  expect(shouldShowTailLoading('running', [])).toBe(true)
})

test('tail loading appears after user-like blocks while the turn is running', () => {
  expect(shouldShowTailLoading('running', [userBlock()])).toBe(true)
  expect(shouldShowTailLoading('running', [pendingSteerBlock()])).toBe(true)
  expect(shouldShowTailLoading('running', [resumeBlock()])).toBe(true)
  expect(shouldShowTailLoading('running', [responseBlock()])).toBe(true)
  expect(shouldShowTailLoading('running', [compactionBoundaryBlock()])).toBe(true)
})

test('tail loading appears after a completed tool while waiting for the model to continue', () => {
  expect(shouldShowTailLoading('running', [toolCallBlock('completed')])).toBe(true)
  expect(shouldShowTailLoading('running', [toolCallBlock('error')])).toBe(true)
})

test('tail loading stays hidden while the tool row itself is executing', () => {
  expect(shouldShowTailLoading('running', [toolCallBlock('executing')])).toBe(false)
})

test('tail loading stays hidden outside running phase', () => {
  expect(shouldShowTailLoading('idle', [])).toBe(false)
  expect(shouldShowTailLoading('idle', [toolCallBlock('completed')])).toBe(false)
  expect(shouldShowTailLoading('compacting', [userBlock()])).toBe(false)
})

test('tail loading stays hidden while assistant content is the latest visible block', () => {
  expect(shouldShowTailLoading('running', [thinkingBlock()])).toBe(false)
  expect(shouldShowTailLoading('running', [textBlock()])).toBe(false)
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

function resumeBlock(): MessageListBlock {
  return { type: 'resume', id: 'resume-1', turnId: 'turn-1', createdAt, model }
}

function responseBlock(): MessageListBlock {
  return {
    type: 'response',
    id: 'response-1',
    createdAt,
    model,
    usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 },
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
    metadata: null,
  }
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

function textBlock(): MessageListBlock {
  return {
    type: 'text',
    id: 'text-1',
    createdAt,
    model,
    text: 'answer',
  }
}
