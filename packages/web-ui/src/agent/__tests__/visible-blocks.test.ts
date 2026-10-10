import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/protocol'
import { compactionSummaryTokens, getVisibleBlocks } from '../visible-blocks'

const createdAt = '1970-01-01T00:00:00.000Z'

test('the transcript hides what the reader never sees: usage, resumes and the hidden inputs', () => {
  const blocks: Block[] = [
    { type: 'user', id: 'user-1', turnId: 'user-1', createdAt, model, content: [{ type: 'text', text: 'go' }], preamble: null },
    tool('call-1', 'shell'),
    { type: 'response', id: 'response-1', createdAt, model, usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 } },
    { type: 'wakeup', id: 'wakeup-1', turnId: 'wakeup-1', createdAt, model, placement: 'new_turn', text: 'Command 17 (Run the tests) ended with exit code 0; look at it with demi shell status 17.' },
    { type: 'context', id: 'context-1', turnId: 'wakeup-1', createdAt, model, source: 'execution', text: 'The target changed.' },
    { type: 'resume', id: 'resume-1', turnId: 'wakeup-1', createdAt, model },
    { type: 'steer', id: 'steer-1', turnId: 'wakeup-1', createdAt, model, content: [{ type: 'text', text: 'also' }] },
    tool('call-2', 'shell'),
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(
    ['user-1', 'call-1', 'steer-1', 'call-2']
  )
})

test('a compaction shows at its marker, where it was triggered, not at its boundary', () => {
  // The user compacted after a2: the summary goes in before a2, the marker after it.
  const blocks: Block[] = [
    user('u1'),
    text('a1'),
    user('u2'),
    { type: 'compaction_boundary', id: 'boundary', createdAt, model, summary: 'summary', summaryTokens: 2_400 },
    text('a2'),
    { type: 'compaction_marker', id: 'marker', createdAt, model, boundaryId: 'boundary', compactedTokens: 90_000 },
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(['u1', 'a1', 'u2', 'a2', 'marker'])
  expect(compactionSummaryTokens(blocks).get('marker')).toBe(2_400)
})

test('a boundary whose marker an edit removed shows the compaction itself', () => {
  const blocks: Block[] = [
    user('u1'),
    { type: 'compaction_boundary', id: 'boundary', createdAt, model, summary: 'summary', summaryTokens: 2_400 },
    text('a1'),
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(['u1', 'boundary', 'a1'])
  expect(compactionSummaryTokens(blocks).get('boundary')).toBe(2_400)
})

function user(id: string): Block {
  return { type: 'user', id, turnId: id, createdAt, model, content: [{ type: 'text', text: id }], preamble: null }
}

function text(id: string): Block {
  return { type: 'text', id, createdAt, model, text: id, forkable: true }
}

function tool(id: string, toolName: string): Extract<Block, {
  type: 'tool_call'
}> {
  return {
    type: 'tool_call',
    id,
    createdAt,
    model,
    toolUseId: id,
    toolName,
    input: '{}',
    status: 'completed',
    output: [],
    view: null,
  }
}

const model: ModelSelection = {
  providerId: 'test',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 1000,
    outputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}
