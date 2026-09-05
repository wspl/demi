import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/core'
import { isTextBlockStreaming, isThinkingBlockStreaming } from '../block-streaming'

const model: ModelSelection = {
  providerId: 'demo',
  model: {
    id: 'demo',
    name: 'Demo',
    contextWindow: 1,
    inputLimit: 1,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

function thinking(): Block {
  return { type: 'thinking', id: 't1', createdAt: '2026-01-01T00:00:00.000Z', model, text: '…', signature: null }
}

function text(): Block {
  return { type: 'text', id: 'a1', createdAt: '2026-01-01T00:00:00.000Z', model, text: 'hello' }
}

test('thinking is live only as the running tail', () => {
  expect(isThinkingBlockStreaming([thinking()], 'running', 0)).toBe(true)
  expect(isThinkingBlockStreaming([thinking()], 'idle', 0)).toBe(false)
  expect(isThinkingBlockStreaming([thinking(), text()], 'running', 0)).toBe(false)
  expect(isThinkingBlockStreaming([thinking(), text()], 'running', 1)).toBe(false)
})

test('assistant text is live only as the running tail', () => {
  expect(isTextBlockStreaming([text()], 'running', 0)).toBe(true)
  expect(isTextBlockStreaming([text()], 'idle', 0)).toBe(false)
  expect(isTextBlockStreaming([text(), thinking()], 'running', 0)).toBe(false)
  expect(isTextBlockStreaming([thinking(), text()], 'running', 1)).toBe(true)
})
