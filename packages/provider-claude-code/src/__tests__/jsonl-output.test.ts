import { expect, test } from 'bun:test'
import { inferenceItemToClaudeMessage, inputMessagesToJsonl, mapClaudeStdoutMessage, requestToInputMessages } from '../index'

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
      { type: 'tool_use', modelId: 'model', toolUseId: 'tool-1', toolName: 'shell_exec', input: { script: 'pwd' } },
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

test('requestToInputMessages skips unsigned assistant thinking for Claude replay', () => {
  const unsignedThinking = { type: 'assistant_thinking' as const, modelId: 'model', text: 'partial reasoning', signature: null }
  const messages = requestToInputMessages({
    modelId: 'model',
    systemPrompt: 'system',
    cwd: '/tmp',
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
    items: [
      { type: 'user_message', content: [{ type: 'text', text: 'resume' }] },
      unsignedThinking,
      { type: 'assistant_text', modelId: 'model', text: 'visible progress' },
      { type: 'assistant_thinking', modelId: 'model', text: 'also partial', signature: '' },
      { type: 'tool_use', modelId: 'model', toolUseId: 'tool-1', toolName: 'shell_exec', input: { script: 'pwd' } },
    ],
  })

  expect(messages).toEqual([
    { type: 'user', message: { role: 'user', content: [{ type: 'text', text: 'resume' }] } },
    {
      type: 'assistant',
      message: {
        role: 'assistant',
        content: [
          { type: 'text', text: 'visible progress' },
          { type: 'tool_use', id: 'tool-1', name: 'mcp__main__shell_exec', input: { script: 'pwd' } },
        ],
      },
    },
  ])
  expect(inputMessagesToJsonl(messages)).not.toContain('"signature":null')
  expect(inputMessagesToJsonl(messages)).not.toContain('"signature":""')
  expect(inferenceItemToClaudeMessage(unsignedThinking)).toBeNull()
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

test('requestToInputMessages preserves unicode, long fields, and mixed tool results', () => {
  const unicode = 'line separator \u2028 snowman \u2603 face \uD83D\uDE00'
  const longText = 'x'.repeat(10_000)
  const messages = requestToInputMessages({
    modelId: 'model',
    systemPrompt: 'system',
    cwd: '/tmp',
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
    items: [
      { type: 'user_message', content: [{ type: 'text', text: `${unicode}\n${longText}` }] },
      {
        type: 'tool_result',
        toolUseId: 'tool-1',
        output: [
          { type: 'text', text: unicode },
          { type: 'image', source: { mediaType: 'image/png', data: 'AQID' } },
        ],
        isError: true,
      },
    ],
  })

  expect(messages[0]).toEqual({
    type: 'user',
    message: { role: 'user', content: [{ type: 'text', text: `${unicode}\n${longText}` }] },
  })
  expect(messages[1]).toEqual({
    type: 'user',
    message: {
      role: 'user',
      content: [
        {
          type: 'tool_result',
          tool_use_id: 'tool-1',
          is_error: true,
          content: `${unicode}\n[image:image/png]`,
        },
      ],
    },
  })
  expect(inputMessagesToJsonl(messages)).toContain(longText)
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

test('mapClaudeStdoutMessage handles empty content and thinking boundary events', () => {
  expect(mapClaudeStdoutMessage({ type: 'assistant', message: { content: [] } })).toEqual({
    events: [],
    terminal: false,
  })
  expect(
    mapClaudeStdoutMessage({
      type: 'assistant',
      message: { content: [{ type: 'text' }, { type: 'thinking' }, { type: 'redacted_thinking' }] },
    }).events,
  ).toEqual([
    { type: 'text_delta', text: '' },
    { type: 'thinking_start' },
    { type: 'thinking_delta', text: '' },
    { type: 'redacted_thinking', data: '' },
  ])
  expect(
    mapClaudeStdoutMessage({
      type: 'stream_event',
      event: { type: 'content_block_delta', delta: { type: 'thinking_delta' } },
    }).events,
  ).toEqual([{ type: 'thinking_delta', text: '' }])
})

test('mapClaudeStdoutMessage preserves provider error codes and result error text', () => {
  expect(
    mapClaudeStdoutMessage({
      type: 'error',
      message: 'context window exceeded',
      code: 'context_length_exceeded',
    }).events,
  ).toEqual([{ type: 'error', message: 'context window exceeded', code: 'context_length_exceeded' }])
  expect(
    mapClaudeStdoutMessage({
      type: 'error',
      message: 'rate limited, try later',
    }).events,
  ).toEqual([{ type: 'error', message: 'rate limited, try later', code: 'rate_limit' }])
  expect(
    mapClaudeStdoutMessage({
      type: 'error',
      message: 'authentication expired',
      code: 'auth_expired',
    }).events,
  ).toEqual([{ type: 'error', message: 'authentication expired', code: 'auth_expired' }])
  expect(
    mapClaudeStdoutMessage({
      type: 'result',
      is_error: true,
      result: 'context window exceeded',
      errors: ['input is too long'],
    }).events[0],
  ).toEqual({ type: 'error', message: 'context window exceeded\ninput is too long', code: 'context_length_exceeded' })
  expect(
    mapClaudeStdoutMessage({
      type: 'result',
      is_error: true,
      result: 'authentication failed',
    }).events[0],
  ).toEqual({ type: 'error', message: 'authentication failed', code: 'auth_expired' })

  expect(
    mapClaudeStdoutMessage({
      type: 'result',
      is_error: true,
      result: 'rate limited',
      errors: ['try later'],
      usage: { inputTokens: 1, outputTokens: 2, cacheReadTokens: 3, cacheWriteTokens: 4 },
    }),
  ).toEqual({
    terminal: true,
    events: [
      { type: 'error', message: 'rate limited\ntry later', code: 'rate_limit' },
      { type: 'response', usage: { inputTokens: 1, outputTokens: 2, cacheReadTokens: 3, cacheWriteTokens: 4 } },
    ],
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
