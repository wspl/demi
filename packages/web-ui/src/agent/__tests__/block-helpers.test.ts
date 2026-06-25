import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demi/core'
import { shellTerminalOutputChunks } from '../block-helpers'

test('shell terminal output prefers structured interleaved metadata chunks', () => {
  const block = tool({
    outputText: [
      'status: exited',
      'stdout:',
      'visible stdout',
      'stderr:',
      'fallback stderr',
      'stderrPath: demi://stderr',
    ].join('\n'),
    metadata: {
      output: {
        chunks: [
          { stream: 'stdout', text: 'first stdout\n' },
          { stream: 'stderr', text: 'first stderr\n' },
          { stream: 'stdout', text: 'second stdout\n' },
        ],
      },
      stdout: {
        delta: 'metadata stdout\n',
      },
      stderr: {
        delta: 'metadata stderr\n',
      },
    },
  })

  expect(shellTerminalOutputChunks(block)).toEqual([
    { stream: 'stdout', text: 'first stdout\n' },
    { stream: 'stderr', text: 'first stderr\n' },
    { stream: 'stdout', text: 'second stdout\n' },
  ])
})

test('shell terminal output falls back to stdout then stderr sections', () => {
  const block = tool({
    outputText: [
      'status: exited',
      'stdout:',
      'visible stdout',
      'stdoutPath: demi://stdout',
      'stderr:',
      'first stderr line',
      'second stderr line',
      'stderrPath: demi://stderr',
      'stderrOffset: 42',
    ].join('\n'),
  })

  expect(shellTerminalOutputChunks(block)).toEqual([
    { stream: 'stdout', text: 'visible stdout' },
    { stream: 'stderr', text: 'first stderr line\nsecond stderr line' },
  ])
})

test('shell terminal output hides empty sections', () => {
  const block = tool({
    outputText: [
      'status: exited',
      'stdout:',
      '(empty)',
      'stdoutPath: demi://stdout',
      'stderr:',
      '(empty)',
      'stderrPath: demi://stderr',
    ].join('\n'),
  })

  expect(shellTerminalOutputChunks(block)).toEqual([])
})

function tool(options: { outputText: string; metadata?: unknown }): Extract<Block, { type: 'tool_call' }> {
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
    output: [{ type: 'text', text: options.outputText }],
    metadata: options.metadata ?? null,
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
