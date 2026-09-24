import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/protocol'
import { shellTerminalOutputChunks, storedShellView } from '../block-helpers'
import type { ShellToolView } from '../block-types'

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

test('the retained files come from the view, with their edits', () => {
  const files = [
    { path: 'a.ts', kind: 'modified' as const, added: 1, removed: 2, edits: [{ kept: true }] },
    { path: 'b.ts', kind: 'added' as const, added: 3, removed: 0, edits: [{ kept: false }, { kept: true }] },
  ]
  const block = tool({ view: shellView({ files }) })

  expect(storedShellView(block)?.files).toEqual(files)
  expect(storedShellView(tool({ view: shellView() }))?.files).toBeUndefined()
  expect(storedShellView(tool({}))).toBeNull()
  expect(storedShellView(tool({ view: { kind: 'yield_wakeup', wakeupId: 'w', durationMs: 1 } }))).toBeNull()
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

function tool(options: { view?: Extract<Block, { type: 'tool_call' }>['view'] }): Extract<Block, { type: 'tool_call' }> {
  return {
    type: 'tool_call',
    id: 'tool-1',
    createdAt: '1970-01-01T00:00:00.000Z',
    model,
    toolUseId: 'tool-1',
    toolName: 'shell_exec',
    input: '{}',
    status: 'completed',
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
