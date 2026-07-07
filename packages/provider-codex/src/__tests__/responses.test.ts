import { expect, test } from 'bun:test'
import type { InferenceRequest, ProviderEvent } from '@demicodes/provider'
import { buildCodexResponsesRequestBody, mapCodexResponseEvents, splitCodexToolUseId, usageFromResponse } from '../responses'
import { parseSseChunk } from '../sse'

test('buildCodexResponsesRequestBody converts inference items, tools, thinking, and cache key', () => {
  const reasoningItem = {
    type: 'reasoning',
    id: 'rs_1',
    encrypted_content: 'encrypted-reasoning',
    summary: [{ text: 'summary' }],
  }
  const body = buildCodexResponsesRequestBody(
    makeRequest([
      { type: 'user_message', content: [{ type: 'text', text: 'hello' }] },
      { type: 'user_steer', turnId: 'turn-1', content: [{ type: 'text', text: 'steer this turn' }] },
      { type: 'assistant_thinking', modelId: 'gpt-5.4', text: 'private', signature: JSON.stringify(reasoningItem) },
      { type: 'assistant_text', modelId: 'gpt-5.4', text: 'visible' },
      { type: 'tool_use', modelId: 'gpt-5.4', toolUseId: 'call_1|fc_1', toolName: 'shell_exec', input: { script: 'pwd' } },
      { type: 'tool_result', toolUseId: 'call_1|fc_1', output: [{ type: 'text', text: '/tmp' }], isError: false },
    ]),
  )

  expect(body).toMatchObject({
    model: 'gpt-5.4',
    instructions: 'system',
    tool_choice: 'auto',
    parallel_tool_calls: true,
    store: false,
    stream: true,
    include: ['reasoning.encrypted_content'],
    prompt_cache_key: 'session-1',
    reasoning: { effort: 'medium', summary: 'auto' },
  })
  expect(body.tools).toEqual([
    {
      type: 'function',
      name: 'shell_exec',
      description: 'Execute shell',
      parameters: { type: 'object' },
      strict: null,
    },
  ])
  expect(body.input).toEqual([
    { role: 'user', content: [{ type: 'input_text', text: 'hello' }] },
    { role: 'user', content: [{ type: 'input_text', text: 'steer this turn' }] },
    reasoningItem,
    expect.objectContaining({
      type: 'message',
      role: 'assistant',
      content: [{ type: 'output_text', text: 'visible', annotations: [] }],
      status: 'completed',
    }),
    { type: 'function_call', id: 'fc_1', call_id: 'call_1', name: 'shell_exec', arguments: '{"script":"pwd"}' },
    { type: 'function_call_output', call_id: 'call_1', output: '/tmp' },
  ])
})

test('buildCodexResponsesRequestBody skips unsigned thinking and encodes tool result images', () => {
  const body = buildCodexResponsesRequestBody(
    makeRequest([
      { type: 'assistant_thinking', modelId: 'gpt-5.4', text: 'unsigned', signature: null },
      {
        type: 'tool_result',
        toolUseId: 'call_1|fc_1',
        output: [
          { type: 'text', text: 'see image' },
          { type: 'image', source: { mediaType: 'image/png', data: 'AQID' } },
        ],
        isError: false,
      },
    ]),
  )

  expect(body.input).toEqual([
    {
      type: 'function_call_output',
      call_id: 'call_1',
      output: [
        { type: 'input_text', text: 'see image' },
        { type: 'input_image', image_url: 'data:image/png;base64,AQID', detail: 'auto' },
      ],
    },
  ])
})

test('buildCodexResponsesRequestBody writes service tier only when selected', () => {
  const standard = buildCodexResponsesRequestBody({ ...makeRequest([]), thinking: null, serviceTierId: null })
  const priority = buildCodexResponsesRequestBody({ ...makeRequest([]), thinking: null, serviceTierId: 'priority' })

  expect(standard).not.toHaveProperty('service_tier')
  expect(priority.service_tier).toBe('priority')
})

test('mapCodexResponseEvents streams thinking, text, tool calls, and usage', async () => {
  const reasoning = { type: 'reasoning' as const, id: 'rs_1', encrypted_content: 'enc', summary: [{ text: 'thought' }] }
  const events: ProviderEvent[] = []

  for await (const event of mapCodexResponseEvents(iter([
    { type: 'response.output_item.added', item: { type: 'reasoning', id: 'rs_1' } },
    { type: 'response.reasoning_summary_text.delta', delta: 'think' },
    { type: 'response.output_item.done', item: reasoning },
    { type: 'response.output_text.delta', delta: 'hello ' },
    { type: 'response.output_item.added', item: { type: 'function_call', id: 'fc_1', call_id: 'call_1', name: 'shell_exec', arguments: '' } },
    { type: 'response.function_call_arguments.delta', item_id: 'fc_1', delta: '{"script":' },
    { type: 'response.function_call_arguments.done', item_id: 'fc_1', arguments: '{"script":"pwd"}' },
    { type: 'response.output_item.done', item: { type: 'function_call', id: 'fc_1', call_id: 'call_1', name: 'shell_exec' } },
    {
      type: 'response.completed',
      response: {
        id: 'resp_1',
        usage: { input_tokens: 100, output_tokens: 7, input_tokens_details: { cached_tokens: 60 } },
      },
    },
  ]))) {
    events.push(event)
  }

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'think' },
    { type: 'thinking_signature', signature: JSON.stringify(reasoning) },
    { type: 'text_delta', text: 'hello ' },
    { type: 'tool_call_requested', toolUseId: 'call_1|fc_1', toolName: 'shell_exec', input: { script: 'pwd' } },
    { type: 'response', usage: { inputTokens: 40, outputTokens: 7, cacheReadTokens: 60, cacheWriteTokens: 0 } },
  ])
})

test('mapCodexResponseEvents emits final reasoning text when no delta was streamed', async () => {
  const events: ProviderEvent[] = []
  const item = { type: 'reasoning' as const, id: 'rs_1', content: [{ text: 'raw reasoning' }], encrypted_content: 'enc' }

  for await (const event of mapCodexResponseEvents(iter([
    { type: 'response.output_item.added', item: { type: 'reasoning', id: 'rs_1' } },
    { type: 'response.output_item.done', item },
  ]))) {
    events.push(event)
  }

  expect(events).toEqual([
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: 'raw reasoning' },
    { type: 'thinking_signature', signature: JSON.stringify(item) },
  ])
})

test('mapCodexResponseEvents maps failed and incomplete responses to provider errors', async () => {
  const events: ProviderEvent[] = []
  for await (const event of mapCodexResponseEvents(iter([
    { type: 'response.failed', response: { error: { code: 'context_length_exceeded', message: 'too long' } } },
    { type: 'response.incomplete', response: { incomplete_details: { reason: 'max_output_tokens' } } },
    { type: 'error', code: 'server_error', message: 'backend failed' },
    { type: 'error', error: { type: 'invalid_request_error', message: 'Invalid prompt_cache_key' }, status: 400 },
    { type: 'error' },
  ]))) {
    events.push(event)
  }

  expect(events).toEqual([
    { type: 'error', message: 'too long', code: 'context_length_exceeded' },
    { type: 'error', message: 'Incomplete response returned, reason: max_output_tokens', code: 'context_length_exceeded' },
    { type: 'error', message: 'backend failed', code: 'server_error' },
    { type: 'error', message: 'Invalid prompt_cache_key', code: 'invalid_request_error' },
    { type: 'error', message: 'Codex stream error', code: null },
  ])
})

test('SSE parser and usage helpers handle provider wire format', () => {
  expect(parseSseChunk('event: ignored\ndata: {"type":"response.created"}\n')).toEqual({ type: 'response.created' })
  expect(parseSseChunk('data: [DONE]\n')).toBeNull()
  expect(splitCodexToolUseId('call_1|fc_1')).toEqual({ callId: 'call_1', itemId: 'fc_1' })
  expect(usageFromResponse({ usage: { input_tokens: 10, output_tokens: 2, input_tokens_details: { cached_tokens: 4 } } })).toEqual({
    inputTokens: 6,
    outputTokens: 2,
    cacheReadTokens: 4,
    cacheWriteTokens: 0,
  })
})

function makeRequest(items: InferenceRequest['items']): InferenceRequest {
  return {
    sessionId: 'session-1',
    turnId: 'turn-1',
    requestId: 'request-1',
    modelId: 'gpt-5.4',
    systemPrompt: 'system',
    cwd: '/tmp',
    items,
    tools: [{ name: 'shell_exec', description: 'Execute shell', inputSchema: { type: 'object' } }],
    thinking: { type: 'effort', effort: 'medium', summary: null },
    cancel: new AbortController().signal,
  }
}

async function* iter<T>(values: T[]): AsyncIterable<T> {
  for (const value of values) yield value
}
