import { expect, test } from 'bun:test'
import { ProviderDataError } from '@demicodes/provider'
import { parseClaudeOutputMessage, readClaudeMessage } from '../output-schemas'
import { controlRequestToToolCall, mapClaudeStdoutMessage } from '../output'

const invalid = [
  null, [], {}, { type: 42 }, { type: 'unknown_terminal' },
  { type: 'assistant', message: { content: {} } },
  { type: 'assistant', message: { content: [{ type: 'text', text: {} }] } },
  { type: 'assistant', message: { content: [{ type: 'thinking' }] } },
  { type: 'assistant', message: { content: [{ type: 'redacted_thinking', data: 42 }] } },
  { type: 'assistant', message: { content: [{ type: 'tool_use', id: '', name: 'tool', input: {} }] } },
  { type: 'stream_event', event: { type: 'content_block_delta', delta: { type: 'text_delta', text: {} } } },
  { type: 'stream_event', event: { type: 'content_block_delta', index: -1, delta: { type: 'text_delta', text: '' } } },
  { type: 'error', message: { secret: 'SYNTHETIC_SECRET' } },
  { type: 'result', is_error: 'false' },
  { type: 'result', errors: [{}] },
  { type: 'result', usage: null },
  { type: 'result', usage: { input_tokens: '3' } },
  { type: 'result', usage: { output_tokens: Infinity } },
  { type: 'result', usage: { cacheReadTokens: -1 } },
  { type: 'result', usage: { iterations: [null] } },
  { type: 'control_request', method: 'tools/call', params: { name: 'tool' } },
  { type: 'control_request', id: '1', method: 'tools/call', params: { name: [] } },
  { type: 'control_request', id: '1', method: 'tools/call', params: { name: 'tool', _meta: { 'claudecode/toolUseId': 42 } } },
  { type: 'control_response', response: { subtype: 'success', request_id: 42 } },
  { type: 'assistant', message: { content: [{ type: 'tool_use', id: 'id', name: 'tool' }] } },
]

test('Claude ingress rejects malformed known messages before mapping or execution', async () => {
  for (const value of invalid) {
    expect(() => parseClaudeOutputMessage(value)).toThrow(ProviderDataError)
    await expect(readClaudeMessage({ next: async () => ({ done: false, value }) }))
      .rejects.toBeInstanceOf(ProviderDataError)
  }
  try {
    parseClaudeOutputMessage(invalid[12])
    throw new Error('expected validation failure')
  } catch (error) {
    expect(String(error)).toContain('message')
    expect(String(error)).not.toContain('SYNTHETIC_SECRET')
  }
})

test('Claude accepts extension fields and explicit empty strings while validating ignored content', () => {
  const message = parseClaudeOutputMessage({
    type: 'assistant', extension: 42,
    message: { content: [{ type: 'text', text: '', extension: true }] },
  })
  expect(mapClaudeStdoutMessage(message).events).toEqual([{ type: 'text_delta', text: '' }])
  expect(mapClaudeStdoutMessage(parseClaudeOutputMessage({ type: 'system', subtype: 'init' })).events)
    .toEqual([])
  expect(mapClaudeStdoutMessage(parseClaudeOutputMessage({ type: 'result' }))).toMatchObject({
    terminal: true,
    events: [{ type: 'response', usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 } }],
  })
})

test('SDK MCP IDs are preserved; notifications do not invent a request ID', () => {
  const sdkMessage = (inner: unknown) => ({
    type: 'control_request', request_id: 'outer',
    request: { subtype: 'mcp_message', server_name: 'main', message: inner },
  })
  expect(() => parseClaudeOutputMessage(sdkMessage({
    method: 'tools/call', params: { name: 'tool' },
  }))).toThrow(ProviderDataError)
  const notification = parseClaudeOutputMessage(sdkMessage({ method: 'notifications/initialized' }))
  expect(mapClaudeStdoutMessage(notification).controlRequest).toBeUndefined()
  const call = mapClaudeStdoutMessage(parseClaudeOutputMessage(sdkMessage({
    id: 0, method: 'tools/call',
    params: { name: 'mcp__main__shell_exec', arguments: null, _meta: { 'claudecode/toolUseId': 'tool-1' } },
  }))).controlRequest
  expect(call).toMatchObject({ id: 0, outerRequestId: 'outer', toolUseId: 'tool-1' })
  expect(controlRequestToToolCall(call!)).toEqual({
    type: 'tool_call_requested', toolUseId: 'tool-1', toolName: 'shell_exec', input: null,
  })
})
