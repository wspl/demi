import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/protocol'
import { getVisibleBlocks } from '../visible-blocks'

const createdAt = '1970-01-01T00:00:00.000Z'

test('the transcript hides what the reader never sees: usage, resumes and the hidden inputs', () => {
  const blocks: Block[] = [
    { type: 'user', id: 'user-1', turnId: 'user-1', createdAt, model, content: [{ type: 'text', text: 'go' }], preamble: null },
    tool('yield-1', 'yield'),
    { type: 'response', id: 'response-1', createdAt, model, usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 } },
    { type: 'wakeup', id: 'wakeup-1', turnId: 'wakeup-1', createdAt, model, placement: 'new_turn' },
    { type: 'context', id: 'context-1', turnId: 'wakeup-1', createdAt, model, text: 'The target changed.' },
    { type: 'resume', id: 'resume-1', turnId: 'wakeup-1', createdAt, model },
    { type: 'steer', id: 'steer-1', turnId: 'wakeup-1', createdAt, model, content: [{ type: 'text', text: 'also' }] },
    tool('status-1', 'shell_status'),
  ]

  expect(getVisibleBlocks(blocks).map((block) => block.id)).toEqual(
    ['user-1', 'yield-1', 'steer-1', 'status-1']
  )
})

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
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}
