import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { applyTranscriptPatches, diffTranscriptBlocks } from '../index'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

test('transcript patches update tool_call blocks whose status and metadata changed in place', () => {
  const previous = [userBlock(), toolCallBlock({ status: 'executing', metadata: null })]
  const next = [
    userBlock(),
    toolCallBlock({
      status: 'completed',
      output: [{ type: 'text', text: 'done' }],
      metadata: { result: 'ok' },
    }),
  ]

  const patches = diffTranscriptBlocks(previous, next)

  expect(patches.map((patch) => patch.op)).toEqual(['remove', 'add'])
  expect(applyTranscriptPatches(previous, patches)).toEqual(next)
})

test('transcript diff handles non-JSON metadata without throwing', () => {
  const previous = [userBlock(), toolCallBlock({ status: 'completed', metadata: { count: 1n } })]
  const next = [userBlock(), toolCallBlock({ status: 'completed', metadata: { count: 2n } })]

  const patches = diffTranscriptBlocks(previous, next)

  expect(applyTranscriptPatches(previous, patches)).toEqual(next)
})

test('transcript diff handles cyclic metadata without throwing', () => {
  const previousMetadata: Record<string, unknown> = { label: 'previous' }
  previousMetadata.self = previousMetadata
  const nextMetadata: Record<string, unknown> = { label: 'next' }
  nextMetadata.self = nextMetadata
  const previous = [userBlock(), toolCallBlock({ status: 'completed', metadata: previousMetadata })]
  const next = [userBlock(), toolCallBlock({ status: 'completed', metadata: nextMetadata })]

  const patches = diffTranscriptBlocks(previous, next)

  expect(patches.map((patch) => patch.op)).toEqual(['remove', 'add'])
  expect(applyTranscriptPatches(previous, patches)).toEqual(next)
})

function userBlock(): Block {
  return {
    type: 'user',
    id: 'user-1',
    turnId: 'turn-1',
    createdAt: '2026-06-17T00:00:00.000Z',
    model,
    content: [{ type: 'text', text: 'hello' }],
    preamble: null,
  }
}

function toolCallBlock(overrides: Partial<Extract<Block, { type: 'tool_call' }>> = {}): Extract<Block, { type: 'tool_call' }> {
  return {
    type: 'tool_call',
    id: 'tool-1',
    createdAt: '2026-06-17T00:00:01.000Z',
    model,
    toolUseId: 'call-1',
    toolName: 'shell_exec',
    input: '{}',
    status: 'executing',
    streamingOutput: [],
    output: [],
    metadata: null,
    ...overrides,
  }
}
