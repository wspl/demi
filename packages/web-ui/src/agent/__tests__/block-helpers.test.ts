import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { shellTerminalOutputChunks } from '../block-helpers'

test('shell terminal output renders the view chunks', () => {
  const block = tool({
    view: {
      kind: 'shell',
      chunks: [
        { stream: 'stdout', text: 'first stdout\n' },
        { stream: 'stderr', text: 'first stderr\n' },
        { stream: 'stdout', text: 'second stdout\n' },
      ],
    },
  })

  expect(shellTerminalOutputChunks(block)).toEqual([
    { stream: 'stdout', text: 'first stdout\n' },
    { stream: 'stderr', text: 'first stderr\n' },
    { stream: 'stdout', text: 'second stdout\n' },
  ])
})

test('shell terminal output skips empty and malformed chunks', () => {
  const block = tool({
    view: {
      kind: 'shell',
      chunks: [
        { stream: 'stdout', text: '' },
        { stream: 'other', text: 'not a stream' },
        'garbage',
        { stream: 'stderr', text: 'kept\n' },
      ],
    },
  })

  expect(shellTerminalOutputChunks(block)).toEqual([{ stream: 'stderr', text: 'kept\n' }])
})

test('shell terminal output is empty without a chunked view', () => {
  expect(shellTerminalOutputChunks(tool({}))).toEqual([])
  expect(shellTerminalOutputChunks(tool({ view: { kind: 'yield_wakeup' } }))).toEqual([])
})

function tool(options: { view?: unknown }): Extract<Block, { type: 'tool_call' }> {
  return {
    type: 'tool_call',
    id: 'tool-1',
    createdAt: '1970-01-01T00:00:00.000Z',
    model,
    toolUseId: 'tool-1',
    toolName: 'shell_exec',
    input: '{}',
    status: 'completed',
    streamingOutput: [],
    output: [{ type: 'text', text: 'status: exited' }],
    view: options.view ?? null,
  }
}

const model: ModelSelection = {
  providerId: 'test',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 1000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}
