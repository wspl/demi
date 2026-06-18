import { expect, test } from 'bun:test'
import { inputMessagesToJsonl, mapClaudeStdoutMessage, requestToInputMessages } from '../index'

test('requestToInputMessages converts inference items to stream-json input messages', () => {
  const messages = requestToInputMessages({
    modelId: 'model',
    systemPrompt: 'system',
    cwd: '/tmp',
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
    items: [
      { type: 'user_message', content: [{ type: 'text', text: 'hi' }] },
      { type: 'assistant_text', modelId: 'model', text: 'hello' },
      {
        type: 'tool_result',
        toolUseId: 'tool-1',
        output: [{ type: 'text', text: 'ok' }],
        isError: false,
      },
    ],
  })

  expect(messages).toHaveLength(3)
  expect(inputMessagesToJsonl(messages).split('\n')).toHaveLength(3)
  expect(messages[2]).toEqual({
    type: 'user',
    message: {
      role: 'user',
      content: [{ type: 'tool_result', tool_use_id: 'tool-1', is_error: false, content: 'ok' }],
    },
  })
})

test('requestToInputMessages groups Claude assistant turns and tool results', () => {
  const messages = requestToInputMessages({
    modelId: 'model',
    systemPrompt: 'system',
    cwd: '/tmp',
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
    items: [
      { type: 'user_message', content: [{ type: 'text', text: 'run checks' }] },
      { type: 'assistant_thinking', modelId: 'model', text: 'I will inspect.', signature: 'sig' },
      { type: 'assistant_text', modelId: 'model', text: 'Checking.' },
      { type: 'tool_use', modelId: 'model', toolUseId: 'tool-1', toolName: 'mcp__main__shell_exec', input: { script: 'pwd' } },
      { type: 'tool_use', modelId: 'model', toolUseId: 'tool-2', toolName: 'mcp__main__shell_exec', input: { script: 'ls' } },
      { type: 'tool_result', toolUseId: 'tool-1', output: [{ type: 'text', text: '/tmp' }], isError: false },
      { type: 'tool_result', toolUseId: 'tool-2', output: [{ type: 'text', text: 'file.txt' }], isError: false },
      { type: 'assistant_text', modelId: 'model', text: 'Done.' },
    ],
  })

  expect(messages).toHaveLength(4)
  expect(messages[1]).toEqual({
    type: 'assistant',
    message: {
      role: 'assistant',
      content: [
        { type: 'thinking', thinking: 'I will inspect.', signature: 'sig' },
        { type: 'text', text: 'Checking.' },
        { type: 'tool_use', id: 'tool-1', name: 'mcp__main__shell_exec', input: { script: 'pwd' } },
        { type: 'tool_use', id: 'tool-2', name: 'mcp__main__shell_exec', input: { script: 'ls' } },
      ],
    },
  })
  expect(messages[2]).toEqual({
    type: 'user',
    message: {
      role: 'user',
      content: [
        { type: 'tool_result', tool_use_id: 'tool-1', is_error: false, content: '/tmp' },
        { type: 'tool_result', tool_use_id: 'tool-2', is_error: false, content: 'file.txt' },
      ],
    },
  })
})

test('requestToInputMessages converts binary media content to Claude base64 sources', () => {
  const messages = requestToInputMessages({
    modelId: 'model',
    systemPrompt: 'system',
    cwd: '/tmp',
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
    items: [
      {
        type: 'user_message',
        content: [
          {
            type: 'image',
            source: { type: 'binary', data: new Uint8Array([1, 2, 3]), mediaType: 'image/png' },
          },
          {
            type: 'image',
            source: { type: 'url', url: 'https://example.com/image.png' },
          },
          {
            type: 'document',
            source: {
              data: new Uint8Array([4, 5, 6]),
              mediaType: 'application/pdf',
              fileName: 'report.pdf',
            },
          },
        ],
      },
    ],
  })

  expect(messages).toEqual([
    {
      type: 'user',
      message: {
        role: 'user',
        content: [
          { type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'AQID' } },
          { type: 'image', source: { type: 'url', url: 'https://example.com/image.png' } },
          {
            type: 'document',
            source: { type: 'base64', media_type: 'application/pdf', data: 'BAUG' },
            title: 'report.pdf',
          },
        ],
      },
    },
  ])
  expect(JSON.parse(inputMessagesToJsonl(messages))).toEqual(messages[0])
})

test('mapClaudeStdoutMessage maps assistant content, stream deltas, tool calls, and usage', () => {
  expect(
    mapClaudeStdoutMessage({
      type: 'assistant',
      message: {
        content: [
          { type: 'text', text: 'hello' },
          { type: 'tool_use', id: 'tool-1', name: 'mcp__main__shell_exec', input: { script: 'pwd' } },
        ],
      },
    }).events,
  ).toEqual([
    { type: 'text_delta', text: 'hello' },
    { type: 'tool_call_requested', toolUseId: 'tool-1', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])

  expect(
    mapClaudeStdoutMessage({
      type: 'stream_event',
      event: { type: 'content_block_delta', delta: { type: 'text_delta', text: 'delta' } },
    }).events,
  ).toEqual([{ type: 'text_delta', text: 'delta' }])

  expect(
    mapClaudeStdoutMessage({
      type: 'stream_event',
      event: { type: 'message_delta', usage: { input_tokens: 1, output_tokens: 1 } },
    }).events,
  ).toEqual([])

  expect(
    mapClaudeStdoutMessage({
      type: 'result',
      usage: { input_tokens: 2, output_tokens: 3, cache_read_input_tokens: 4, cache_creation_input_tokens: 5 },
    }),
  ).toEqual({
    events: [{ type: 'response', usage: { inputTokens: 2, outputTokens: 3, cacheReadTokens: 4, cacheWriteTokens: 5 } }],
    terminal: true,
  })
})

test('mapClaudeStdoutMessage rejects malformed assistant tool_use blocks', () => {
  expect(
    mapClaudeStdoutMessage({
      type: 'assistant',
      message: {
        content: [
          { type: 'tool_use', id: 'tool-1', input: { script: 'pwd' } },
          { type: 'tool_use', name: 'mcp__main__shell_exec', input: { script: 'pwd' } },
        ],
      },
    }).events,
  ).toEqual([
    { type: 'error', message: 'Invalid tool_use block from Claude Code', code: null },
    { type: 'error', message: 'Invalid tool_use block from Claude Code', code: null },
  ])
})
