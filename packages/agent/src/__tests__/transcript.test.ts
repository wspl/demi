import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { events } from '@demicodes/provider/testing'
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

  transcript.pushUserTurn('test-turn', model, [{ type: 'text', text: 'hello' }], 'preamble')
  transcript.applyProviderEvent(model, events.text('hi '))
  transcript.applyProviderEvent(model, events.text('there'))
  transcript.applyProviderEvent(model, events.response({ inputTokens: 2, outputTokens: 3 }))

  expect(transcript.blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  expect(transcript.blocks[1]).toMatchObject({ type: 'text', text: 'hi there' })

  const items = transcript.collectInferenceItems()
  expect(items).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'hello' }],
    },
    { type: 'assistant_text', modelId: 'test-model', text: 'hi there' },
  ])
})

test('Transcript appends steer blocks and replays them in turn order', () => {
  const transcript = makeTranscript()

  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'start' }], 'preamble')
  const steer = transcript.pushSteer('turn-1', model, [{ type: 'text', text: 'prefer concise output' }])
  transcript.applyProviderEvent(model, events.text('ok'))

  expect(steer).toMatchObject({
    type: 'steer',
    id: 'b2',
    turnId: 'turn-1',
    content: [{ type: 'text', text: 'prefer concise output' }],
  })
  expect(transcript.blocks.map((block) => block.type)).toEqual(['user', 'steer', 'text'])
  expect(transcript.collectInferenceItems()).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'start' }],
    },
    {
      type: 'user_steer',
      turnId: 'turn-1',
      content: [{ type: 'text', text: 'prefer concise output' }],
    },
    { type: 'assistant_text', modelId: 'test-model', text: 'ok' },
  ])
})

test('Transcript completes pending tool calls and emits exact tool inference items', () => {
  const transcript = makeTranscript()

  transcript.applyProviderEvent(
    model,
    events.toolCall('tool-1', 'shell_exec', {
      script: 'echo hi',
      cwd: '/workspace',
      env: { NAME: 'demi' },
    }),
  )
  expect(transcript.findPendingToolUseId()).toBe('tool-1')
  expect(transcript.pendingToolCalls()).toHaveLength(1)

  const completed = transcript.completeToolCall(
    'tool-1',
    [
      { type: 'text', text: 'hi' },
      { type: 'image', source: { mediaType: 'image/png', data: 'base64-image' } },
    ],
    false,
    { durationMs: 5, private: 'not model visible' },
  )

  expect(completed).toMatchObject({ type: 'tool_call', status: 'completed' })
  expect(transcript.pendingToolCalls()).toHaveLength(0)
  expect(transcript.collectInferenceItems()).toEqual([
    {
      type: 'tool_use',
      modelId: 'test-model',
      toolUseId: 'tool-1',
      toolName: 'shell_exec',
      input: {
        script: 'echo hi',
        cwd: '/workspace',
        env: { NAME: 'demi' },
      },
    },
    {
      type: 'tool_result',
      toolUseId: 'tool-1',
      output: [
        { type: 'text', text: 'hi' },
        { type: 'image', source: { mediaType: 'image/png', data: 'base64-image' } },
      ],
      isError: false,
    },
  ])
})

test('Transcript completes the pending tool call when tool ids repeat', () => {
  const transcript = makeTranscript()

  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'shell_exec', { script: 'printf first' }))
  transcript.completeToolCall('tool-1', [{ type: 'text', text: 'first' }])
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'shell_exec', { script: 'printf second' }))

  const completed = transcript.completeToolCall('tool-1', [{ type: 'text', text: 'second' }])

  expect(completed).toMatchObject({ type: 'tool_call', status: 'completed', output: [{ type: 'text', text: 'second' }] })
  const toolBlocks = transcript.blocks.filter((block) => block.type === 'tool_call')
  expect(toolBlocks).toHaveLength(2)
  expect(toolBlocks[0]).toMatchObject({ type: 'tool_call', status: 'completed', output: [{ type: 'text', text: 'first' }] })
  expect(toolBlocks[1]).toMatchObject({ type: 'tool_call', status: 'completed', output: [{ type: 'text', text: 'second' }] })
  expect(transcript.pendingToolCalls()).toHaveLength(0)
})

test('Transcript replays thinking signatures and redacted thinking in provider order', () => {
  const transcript = makeTranscript()

  transcript.pushUserTurn('test-turn', model, [{ type: 'text', text: 'think through this' }])
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

  transcript.pushUserTurn('test-turn', model, [{ type: 'text', text: 'old question' }])
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, [{ type: 'text', text: 'recent question' }])
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

  transcript.pushUserTurn('test-turn', model, [{ type: 'text', text: 'old question' }], 'preamble')
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
  transcript.pushUserTurn('test-turn', model, [{ type: 'text', text: 'recent question' }])
  transcript.pushSteer('test-turn', model, [{ type: 'text', text: 'with extra constraint' }])
  transcript.appendExtensionStateSnapshot('todo', { items: [{ id: 'T1', text: 'persist me' }] })
  transcript.appendCompactionMarker(model, boundary.id, 123)
  transcript.pushResumeTurn('test-turn', model)

  const snapshot = transcript.snapshot()
  const parsed = JSON.parse(JSON.stringify(snapshot)) as typeof snapshot
  const restored = new Transcript(parsed.blocks)

  expect(parsed).toEqual(snapshot)
  expect(restored.snapshot()).toEqual(snapshot)
  expect(restored.blocks.map((block) => block.type)).toEqual(transcript.blocks.map((block) => block.type))
  expect(restored.collectInferenceItems()).toEqual(transcript.collectInferenceItems())
  expect(restored.latestExtensionStateSnapshot('todo')).toEqual(transcript.latestExtensionStateSnapshot('todo'))
})

test('Transcript snapshot is insulated from later live block mutations', () => {
  const transcript = makeTranscript()

  transcript.applyProviderEvent(model, events.toolCall('tool-snapshot', 'shell_exec', { script: 'printf hi' }))
  const snapshot = transcript.snapshot()

  transcript.completeToolCall('tool-snapshot', [{ type: 'text', text: 'hi' }])

  expect(snapshot.blocks[0]).toMatchObject({
    type: 'tool_call',
    status: 'executing',
    output: [],
  })

  const snapshotTool = snapshot.blocks[0]
  if (snapshotTool?.type !== 'tool_call') throw new Error('expected tool_call snapshot')
  snapshotTool.output.push({ type: 'text', text: 'snapshot-only' })

  expect(transcript.blocks[0]).toMatchObject({
    type: 'tool_call',
    status: 'completed',
    output: [{ type: 'text', text: 'hi' }],
  })
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

test('context estimate anchors on the latest provider-reported usage', () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'x'.repeat(40_000) }])
  transcript.applyProviderEvent(model, { type: 'text_delta', text: 'reply' })
  transcript.applyProviderEvent(model, {
    type: 'response',
    usage: { inputTokens: 1_234, outputTokens: 50, cacheReadTokens: 100, cacheWriteTokens: 0 },
  })

  // Anchored: the reported usage replaces the (much larger) char estimate.
  expect(transcript.estimateContextTokens()).toBe(1_384)

  // Blocks streamed after the anchor add their char estimate on top.
  transcript.pushUserTurn('turn-2', model, [{ type: 'text', text: 'y'.repeat(4_000) }])
  expect(transcript.estimateContextTokens()).toBe(1_384 + 1_000)
})

test('context estimate rejects a provider usage anchor larger than the model window', () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'small question' }])
  transcript.applyProviderEvent(model, { type: 'text_delta', text: 'small reply' })
  transcript.applyProviderEvent(model, {
    type: 'response',
    usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 1_000_000, cacheWriteTokens: 0 },
  })

  expect(transcript.estimateContextTokens(100_000)).toBeLessThan(100)
})

test('compaction after the last response invalidates the usage anchor', () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('turn-1', model, [{ type: 'text', text: 'x'.repeat(8_000) }])
  transcript.applyProviderEvent(model, {
    type: 'response',
    usage: { inputTokens: 999_999, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 },
  })
  const boundary = transcript.insertCompactionBoundary(0, model, 'short summary', 4)
  transcript.appendCompactionMarker(model, boundary.id, 2_000)

  // The stale usage measured pre-compaction history; fall back to estimation
  // of the replay window (boundary summary + kept blocks).
  expect(transcript.estimateContextTokens()).toBeLessThan(10_000)
})

test('images and documents contribute to the unanchored context estimate', () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('turn-1', model, [
    { type: 'text', text: 'look' },
    { type: 'image', source: { type: 'binary', data: new Uint8Array(3_000_000), mediaType: 'image/png' } },
    { type: 'document', source: { data: new Uint8Array(40_000), mediaType: 'application/pdf', fileName: 'doc.pdf' } },
  ])

  const estimate = transcript.estimateContextTokens()
  // 3MB image ~= 3000 tokens, 40KB document ~= 10000 tokens.
  expect(estimate).toBeGreaterThan(12_000)
})
