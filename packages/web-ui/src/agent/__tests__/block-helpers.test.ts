import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import type { ShellToolView } from '@demicodes/agent'
import { shellFileChanges, shellTerminalOutputChunks } from '../block-helpers'

test('shell terminal output renders the view chunks', () => {
  const block = tool({
    view: shellView({
      chunks: [
        { stream: 'stdout', text: 'first stdout\n' },
        { stream: 'stderr', text: 'first stderr\n' },
        { stream: 'stdout', text: 'second stdout\n' },
      ],
    }),
  })

  expect(shellTerminalOutputChunks(block)).toEqual([
    { stream: 'stdout', text: 'first stdout\n' },
    { stream: 'stderr', text: 'first stderr\n' },
    { stream: 'stdout', text: 'second stdout\n' },
  ])
})

test('a view that does not match the shell contract shows nothing', () => {
  const malformedChunk = tool({
    view: { ...shellView(), chunks: [{ stream: 'other', text: 'not a stream' }] },
  })
  const missingField = tool({
    view: { kind: 'shell', chunks: [{ stream: 'stdout', text: 'orphan\n' }] },
  })

  expect(shellTerminalOutputChunks(malformedChunk)).toEqual([])
  expect(shellTerminalOutputChunks(missingField)).toEqual([])
  expect(shellTerminalOutputChunks(tool({}))).toEqual([])
  expect(shellTerminalOutputChunks(tool({ view: { kind: 'yield_wakeup' } }))).toEqual(
    []
  )
})

test('shell file changes come from the view', () => {
  const block = tool({
    view: shellView({
      files: [
        { path: 'a.ts', kind: 'modified', added: 1, removed: 2 },
        { path: 'b.ts', kind: 'renamed', from: 'c.ts', added: 0, removed: 0 },
      ],
    }),
  })

  expect(shellFileChanges(block)).toEqual([
    { path: 'a.ts', kind: 'modified', added: 1, removed: 2 },
    { path: 'b.ts', kind: 'renamed', from: 'c.ts', added: 0, removed: 0 },
  ])
  expect(shellFileChanges(tool({ view: shellView() }))).toEqual([])
  expect(shellFileChanges(tool({}))).toEqual([])
})

test('a malformed file entry discards the view it is in', () => {
  const block = tool({
    view: {
      ...shellView(),
      files: [
        { path: 'a.ts', kind: 'modified', added: 1, removed: 2 },
        { path: 'd.ts', kind: 'touched', added: 1, removed: 0 },
      ],
    },
  })

  expect(shellFileChanges(block)).toEqual([])
})

function shellView(overrides: Partial<ShellToolView> = {}): ShellToolView {
  return {
    kind: 'shell',
    status: 'exited',
    shellId: 'shell-1',
    commandId: 'cmd-1',
    exitCode: 0,
    runningMs: 12,
    idleMs: 0,
    chunks: [],
    viewTruncated: false,
    ...overrides,
  }
}

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
