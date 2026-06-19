import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { events } from '@demi/provider'
import { Transcript } from '../index'

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

function makeTranscript(): Transcript {
  let id = 0
  return new Transcript([], {
    idFactory: () => `b${++id}`,
    now: () => '2026-06-17T00:00:00.000Z',
  })
}

test('Transcript appends user turns and provider text/response events', () => {
  const transcript = makeTranscript()

  transcript.pushUserTurn(model, [{ type: 'text', text: 'hello' }], 'preamble')
  transcript.applyProviderEvent(model, events.text('hi '))
  transcript.applyProviderEvent(model, events.text('there'))
  transcript.applyProviderEvent(model, events.response({ inputTokens: 2, outputTokens: 3 }))

  expect(transcript.blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  expect(transcript.blocks[1]).toMatchObject({ type: 'text', text: 'hi there' })

  const items = transcript.collectInferenceItems()
  expect(items.map((item) => item.type)).toEqual(['user_message', 'assistant_text'])
})

test('Transcript completes pending tool calls and emits tool result inference items', () => {
  const transcript = makeTranscript()

  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'shell_exec', { script: 'echo hi' }))
  expect(transcript.findPendingToolUseId()).toBe('tool-1')
  expect(transcript.pendingToolCalls()).toHaveLength(1)

  const completed = transcript.completeToolCall('tool-1', [{ type: 'text', text: 'hi' }])

  expect(completed).toMatchObject({ type: 'tool_call', status: 'completed' })
  expect(transcript.pendingToolCalls()).toHaveLength(0)
  expect(transcript.collectInferenceItems().map((item) => item.type)).toEqual(['tool_use', 'tool_result'])
})

test('Transcript replays thinking signatures and redacted thinking in provider order', () => {
  const transcript = makeTranscript()

  transcript.pushUserTurn(model, [{ type: 'text', text: 'think through this' }])
  transcript.applyProviderEvent(model, { type: 'thinking_start' })
  transcript.applyProviderEvent(model, { type: 'thinking_delta', text: 'private ' })
  transcript.applyProviderEvent(model, { type: 'thinking_delta', text: 'notes' })
  transcript.applyProviderEvent(model, { type: 'thinking_signature', signature: 'sig-1' })
  transcript.applyProviderEvent(model, { type: 'redacted_thinking', data: 'opaque-redacted-data' })
  transcript.applyProviderEvent(model, events.text('visible answer'))

  expect(transcript.blocks.map((block) => block.type)).toEqual(['user', 'thinking', 'redacted_thinking', 'text'])
  expect(transcript.collectInferenceItems()).toEqual([
    { type: 'user_message', content: [{ type: 'text', text: 'think through this' }] },
    { type: 'assistant_thinking', modelId: 'test-model', text: 'private notes', signature: 'sig-1' },
    { type: 'assistant_redacted_thinking', modelId: 'test-model', data: 'opaque-redacted-data' },
    { type: 'assistant_text', modelId: 'test-model', text: 'visible answer' },
  ])
})

test('Transcript safely stores non-JSON tool inputs', () => {
  const transcript = makeTranscript()
  const circular: Record<string, unknown> = { id: 1n }
  circular.self = circular

  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'shell_exec', undefined))
  transcript.applyProviderEvent(model, events.toolCall('tool-2', 'shell_exec', circular))

  expect(transcript.blocks[0]).toMatchObject({ type: 'tool_call', input: 'null' })
  expect(transcript.collectInferenceItems()).toMatchObject([
    { type: 'tool_use', input: null },
    { type: 'tool_use', input: { id: '1', self: '[Circular]' } },
  ])
})

test('Transcript removes dangling executing tool calls', () => {
  const transcript = makeTranscript()

  transcript.applyProviderEvent(model, events.toolCall('done', 'shell_exec', { script: 'true' }))
  transcript.completeToolCall('done', [{ type: 'text', text: '' }])
  transcript.applyProviderEvent(model, events.toolCall('dangling', 'shell_exec', { script: 'sleep 1' }))

  const removed = transcript.removeDanglingToolCalls()

  expect(removed.map((block) => block.type)).toEqual(['tool_call'])
  expect(transcript.blocks).toHaveLength(1)
  expect(transcript.blocks[0]).toMatchObject({ type: 'tool_call', toolUseId: 'done' })
})

test('Transcript inserts compaction boundary and replays from the latest boundary', () => {
  const transcript = makeTranscript()

  transcript.pushUserTurn(model, [{ type: 'text', text: 'old question' }])
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, [{ type: 'text', text: 'recent question' }])
  transcript.applyProviderEvent(model, events.text('recent answer'))

  const cutPoint = transcript.findCompactionCutPoint(1)
  expect(cutPoint).toBe(4)

  const boundary = transcript.insertCompactionBoundary(cutPoint ?? 0, model, 'old summary', 10)
  transcript.appendCompactionMarker(model, boundary.id, 42)

  expect(transcript.findLastCompactionIndex()).toBe(4)
  expect(transcript.replayableBlocks().map((block) => block.type)).toEqual([
    'compaction_boundary',
    'text',
    'compaction_marker',
  ])

  const items = transcript.collectInferenceItems()
  expect(items[0]).toEqual({
    type: 'user_message',
    content: [{ type: 'text', text: 'Previous conversation summary:\nold summary' }],
  })
  expect(items.map((item) => item.type)).toEqual(['user_message', 'assistant_text'])
})

test('Transcript snapshot survives JSON roundtrip without changing replay or metadata', () => {
  const transcript = makeTranscript()

  transcript.pushUserTurn(model, [{ type: 'text', text: 'old question' }], 'preamble')
  transcript.applyProviderEvent(model, { type: 'thinking_start' })
  transcript.applyProviderEvent(model, { type: 'thinking_delta', text: 'private notes' })
  transcript.applyProviderEvent(model, { type: 'thinking_signature', signature: 'sig-json' })
  transcript.applyProviderEvent(model, { type: 'redacted_thinking', data: 'redacted-json' })
  transcript.applyProviderEvent(model, events.toolCall('tool-json', 'read_file', { path: 'src/a.ts' }))
  transcript.completeToolCall(
    'tool-json',
    [
      { type: 'text', text: 'file text' },
      { type: 'image', source: { mediaType: 'image/png', data: 'base64-image' } },
    ],
    false,
    { bytes: 42, paths: ['src/a.ts'] },
  )
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response({ inputTokens: 10, outputTokens: 2, cacheReadTokens: 3 }))
  transcript.pushAbort(model, true)
  const boundary = transcript.insertCompactionBoundary(transcript.blocks.length, model, 'json summary', 3)
  transcript.pushUserTurn(model, [{ type: 'text', text: 'recent question' }])
  transcript.appendExtensionStateSnapshot('todo', { items: [{ id: 'T1', text: 'persist me' }] })
  transcript.appendCompactionMarker(model, boundary.id, 123)
  transcript.pushResumeTurn(model)

  const snapshot = transcript.snapshot()
  const parsed = JSON.parse(JSON.stringify(snapshot)) as typeof snapshot
  const restored = new Transcript(parsed.blocks)

  expect(parsed).toEqual(snapshot)
  expect(restored.snapshot()).toEqual(snapshot)
  expect(restored.blocks.map((block) => block.type)).toEqual(transcript.blocks.map((block) => block.type))
  expect(restored.collectInferenceItems()).toEqual(transcript.collectInferenceItems())
  expect(restored.latestExtensionStateSnapshot('todo')).toEqual(transcript.latestExtensionStateSnapshot('todo'))
})

test('Transcript returns the latest extension state snapshot', () => {
  const transcript = makeTranscript()

  transcript.appendExtensionStateSnapshot('todo', { count: 1 })
  transcript.appendExtensionStateSnapshot('other', { enabled: true })
  transcript.appendExtensionStateSnapshot('todo', { count: 2 })

  expect(transcript.latestExtensionStateSnapshot('todo')).toMatchObject({
    type: 'extension_state_snapshot',
    extensionName: 'todo',
    state: { count: 2 },
  })
  expect(transcript.latestExtensionStateSnapshot()).toMatchObject({
    type: 'extension_state_snapshot',
    extensionName: 'todo',
  })
})

test('Transcript token estimates tolerate non-JSON extension state', () => {
  const transcript = makeTranscript()
  const state: Record<string, unknown> = { count: 1n }
  state.self = state

  transcript.appendExtensionStateSnapshot('todo', state)

  expect(transcript.estimateContextTokens()).toBeGreaterThan(0)
})
