import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { shellFileChanges, shellTerminalOutputChunks } from '../block-helpers'

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

  expect(shellTerminalOutputChunks(block)).toEqual(
    [{ stream: 'stderr', text: 'kept\n' }]
  )
})

test('shell terminal output is empty without a chunked view', () => {
  expect(shellTerminalOutputChunks(tool({}))).toEqual([])
  expect(shellTerminalOutputChunks(tool({ view: { kind: 'yield_wakeup' } }))).toEqual(
    []
  )
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
    outputLimit: null,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
  serviceTierId: null,
}

test('shell file changes come from the view and drop malformed entries', () => {
  const block = tool({
    view: {
      kind: 'shell',
      chunks: [],
      files: [
        { path: 'a.ts', kind: 'modified', added: 1, removed: 2 },
        { path: 'b.ts', kind: 'renamed', from: 'c.ts', added: 0, removed: 0 },
        { path: '', kind: 'added', added: 1, removed: 0 },
        { path: 'd.ts', kind: 'touched', added: 1, removed: 0 },
        { path: 'e.ts', kind: 'added', added: '1', removed: 0 },
        'f.ts',
      ],
    },
  })
  expect(shellFileChanges(block)).toEqual([
    { path: 'a.ts', kind: 'modified', added: 1, removed: 2 },
    { path: 'b.ts', kind: 'renamed', from: 'c.ts', added: 0, removed: 0 },
  ])
  expect(shellFileChanges(tool({}))).toEqual([])
})
