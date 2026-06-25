import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demi/core'
import { shellStderrText } from '../block-helpers'

test('shell stderr text prefers structured metadata', () => {
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
      stderr: {
        delta: 'metadata stderr\n',
      },
    },
  })

  expect(shellStderrText(block)).toBe('metadata stderr\n')
})

test('shell stderr text falls back to the formatted stderr section', () => {
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

  expect(shellStderrText(block)).toBe('first stderr line\nsecond stderr line')
})

test('shell stderr text hides empty stderr sections', () => {
  const block = tool({
    outputText: [
      'status: exited',
      'stdout:',
      'visible stdout',
      'stdoutPath: demi://stdout',
      'stderr:',
      '(empty)',
      'stderrPath: demi://stderr',
    ].join('\n'),
  })

  expect(shellStderrText(block)).toBe('')
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
