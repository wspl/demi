import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { AgentSession, type AgentHarnessRuntime } from '@demi/agent'
import { BashEnvironment, createShellSessionTools } from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
import type { InferenceRequest } from '@demi/provider'
import {
  ClaudeCodeProvider,
  createClaudeCodeProviderDefinition,
  parseClaudeCodeProviderConfig,
  type ClaudeTransport,
  type ClaudeTransportFactory,
} from '../index'

const model: ModelSelection = {
  providerId: 'claude-code',
  model: {
    id: 'claude-test',
    name: 'Claude Test',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

function makeRequest(items: InferenceRequest['items'] = []): InferenceRequest {
  return {
    sessionId: 'test-session',
    turnId: 'test-turn',
    requestId: 'test-request',
    modelId: 'claude-test',
    systemPrompt: 'system',
    cwd: '/tmp',
    items,
    tools: [
      {
        name: 'shell_exec',
        description: 'Execute shell',
        inputSchema: { type: 'object' },
      },
    ],
    thinking: null,
    cancel: new AbortController().signal,
  }
}

function makeRequestWithoutTools(items: InferenceRequest['items'] = []): InferenceRequest {
  return { ...makeRequest(items), tools: [] }
}

test('Claude Code provider definition only accepts serializable config fields', async () => {
  const injectedFactory = fakeFactory(new FakeClaudeTransport([]))
  expect(
    parseClaudeCodeProviderConfig({
      claudePath: '/usr/local/bin/claude',
      maxBudgetUsd: '0.01',
      transportFactory: injectedFactory,
    }),
  ).toEqual({ claudePath: '/usr/local/bin/claude', maxBudgetUsd: '0.01' })
  expect(parseClaudeCodeProviderConfig(undefined)).toEqual({})
  expect(parseClaudeCodeProviderConfig(null)).toEqual({})
  expect(() => parseClaudeCodeProviderConfig(1)).toThrow('must be an object')
  expect(() => parseClaudeCodeProviderConfig({ claudePath: 1 })).toThrow('claudePath')
  expect(() => parseClaudeCodeProviderConfig({ maxBudgetUsd: {} })).toThrow('maxBudgetUsd')

  const provider = await createClaudeCodeProviderDefinition().createProvider({
    transportFactory: injectedFactory,
    claudePath: '/usr/local/bin/claude',
  })
  expect((provider as unknown as { transportFactory?: unknown }).transportFactory).not.toBe(injectedFactory)
})

test('ClaudeCodeProvider streams text and response events from transport messages', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'assistant', message: { content: [{ type: 'text', text: 'hello' }] } },
    { type: 'result', usage: { input_tokens: 1, output_tokens: 2 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const events = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    events.push(event)
  }

  expect(events).toEqual([
    { type: 'text_delta', text: 'hello' },
    { type: 'response', usage: { inputTokens: 1, outputTokens: 2, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
  expect(transport.writes.find((write) => isRecord(write) && write.type === 'user')).toMatchObject({ type: 'user' })
  expect(transport.waitCalls).toBe(1)
})

test('ClaudeCodeProvider preserves cache usage fields from result messages', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'assistant', message: { content: [{ type: 'text', text: 'cached' }] } },
    {
      type: 'result',
      usage: {
        input_tokens: 10,
        output_tokens: 2,
        cache_read_input_tokens: 7,
        cache_creation_input_tokens: 3,
      },
    },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const received = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    received.push(event)
  }

  expect(received).toEqual([
    { type: 'text_delta', text: 'cached' },
    { type: 'response', usage: { inputTokens: 10, outputTokens: 2, cacheReadTokens: 7, cacheWriteTokens: 3 } },
  ])
})

test('ClaudeCodeProvider handles empty successful streams without leaking transport state', async () => {
  const first = new FakeClaudeTransport([])
  const second = new FakeClaudeTransport([
    { type: 'assistant', message: { content: [{ type: 'text', text: 'next run' }] } },
    { type: 'result', usage: { input_tokens: 1 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: sequenceFactory([first, second]) })

  const emptyEvents = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'empty' }] }]))) {
    emptyEvents.push(event)
  }
  expect(emptyEvents).toEqual([])
  expect(first.waitCalls).toBe(1)

  const nextEvents = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'again' }] }]))) {
    nextEvents.push(event)
  }
  expect(nextEvents).toEqual([
    { type: 'text_delta', text: 'next run' },
    { type: 'response', usage: { inputTokens: 1, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider forwards context overflow result errors with usage', async () => {
  const transport = new FakeClaudeTransport([
    {
      type: 'result',
      is_error: true,
      result: 'context window exceeded',
      errors: ['input is too long'],
      usage: { input_tokens: 200_000, output_tokens: 0 },
    },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const received = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'huge' }] }]))) {
    received.push(event)
  }

  expect(received).toEqual([
    { type: 'error', message: 'context window exceeded\ninput is too long', code: 'context_length_exceeded' },
    { type: 'response', usage: { inputTokens: 200_000, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider handles control_request tool calls across run calls', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'control_request', id: 'init-1', method: 'initialize', params: {} },
    { type: 'control_request', id: 'ping-1', method: 'ping', params: {} },
    { type: 'control_request', id: 'list-1', method: 'tools/list', params: {} },
    { type: 'control_request', id: 'call-1', method: 'tools/call', params: { name: 'mcp__main__shell_exec', arguments: { script: 'pwd' } } },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'after tool' }] } },
    { type: 'result', usage: { input_tokens: 1 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const firstEvents = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    firstEvents.push(event)
  }

  expect(firstEvents).toMatchObject([
    { type: 'tool_call_requested', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])
  const firstToolUseId = firstEvents[0]?.type === 'tool_call_requested' ? firstEvents[0].toolUseId : null
  expect(firstToolUseId).toMatch(/^mcp-control-/)
  expect(transport.writes).toContainEqual({
    type: 'control_response',
    id: 'list-1',
    response: {
      tools: [{ name: 'shell_exec', description: 'Execute shell', inputSchema: { type: 'object' } }],
    },
  })
  expect(transport.writes).toContainEqual({
    type: 'control_response',
    id: 'ping-1',
    response: {},
  })

  const secondEvents = []
  for await (const event of provider.run(
    makeRequest([
      {
        type: 'tool_result',
        toolUseId: firstToolUseId ?? '',
        output: [{ type: 'text', text: '/tmp' }],
        isError: false,
      },
    ]),
  )) {
    secondEvents.push(event)
  }

  expect(transport.writes).toContainEqual({
    type: 'control_response',
    id: 'call-1',
    response: { content: '/tmp', isError: false },
  })
  expect(secondEvents).toEqual([
    { type: 'text_delta', text: 'after tool' },
    { type: 'response', usage: { inputTokens: 1, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider handles SDK MCP control_request tool calls across run calls', async () => {
  const transport = new FakeClaudeTransport([
    sdkMcpRequest('list-sdk', 'list-1', 'tools/list'),
    sdkMcpRequest('call-sdk', 'call-1', 'tools/call', {
      name: 'shell_exec',
      arguments: { script: 'pwd' },
    }),
    { type: 'assistant', message: { content: [{ type: 'text', text: 'after sdk tool' }] } },
    { type: 'result', usage: { input_tokens: 2, output_tokens: 4 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const firstEvents = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    firstEvents.push(event)
  }

  expect(firstEvents).toMatchObject([
    { type: 'tool_call_requested', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])
  const firstToolUseId = firstEvents[0]?.type === 'tool_call_requested' ? firstEvents[0].toolUseId : null
  expect(firstToolUseId).toMatch(/^mcp-control-/)
  expect(findSdkMcpResponse(transport.writes, 'list-sdk').result).toEqual({
    tools: [{ name: 'shell_exec', description: 'Execute shell', inputSchema: { type: 'object' } }],
  })

  const secondEvents = []
  for await (const event of provider.run(
    makeRequest([
      {
        type: 'tool_result',
        toolUseId: firstToolUseId ?? '',
        output: [{ type: 'text', text: '/tmp' }],
        isError: false,
      },
    ]),
  )) {
    secondEvents.push(event)
  }

  expect(findSdkMcpResponse(transport.writes, 'call-sdk').result).toEqual({
    content: [{ type: 'text', text: '/tmp' }],
    isError: false,
  })
  expect(secondEvents).toEqual([
    { type: 'text_delta', text: 'after sdk tool' },
    { type: 'response', usage: { inputTokens: 2, outputTokens: 4, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider replays internal tool names with MCP names on fresh runs', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'assistant', message: { content: [{ type: 'text', text: 'ready' }] } },
    { type: 'result', usage: { input_tokens: 3, output_tokens: 1 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const events = []
  for await (const event of provider.run(
    makeRequest([
      { type: 'user_message', content: [{ type: 'text', text: 'previous work' }] },
      {
        type: 'tool_use',
        modelId: 'claude-test',
        toolUseId: 'tool-1',
        toolName: 'shell_exec',
        input: { script: 'pwd' },
      },
      {
        type: 'tool_result',
        toolUseId: 'tool-1',
        output: [{ type: 'text', text: '/tmp' }],
        isError: false,
      },
      { type: 'user_message', content: [{ type: 'text', text: 'continue' }] },
    ]),
  )) {
    events.push(event)
  }

  const assistantWrite = transport.writes.find((write): write is { type: 'assistant'; message: { content: unknown[] } } => {
    return isRecord(write) && write.type === 'assistant' && isRecord(write.message) && Array.isArray(write.message.content)
  })
  expect(assistantWrite?.message.content).toContainEqual({
    type: 'tool_use',
    id: 'tool-1',
    name: 'mcp__main__shell_exec',
    input: { script: 'pwd' },
  })
  expect(events).toEqual([
    { type: 'text_delta', text: 'ready' },
    { type: 'response', usage: { inputTokens: 3, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider rejects malformed SDK MCP tools/call without entering pending state', async () => {
  const transport = new FakeClaudeTransport([
    sdkMcpRequest('call-sdk', 'call-1', 'tools/call', { arguments: { script: 'pwd' } }),
    { type: 'assistant', message: { content: [{ type: 'text', text: 'continued after malformed sdk call' }] } },
    { type: 'result', usage: { input_tokens: 3, output_tokens: 1 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const events = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    events.push(event)
  }

  expect(findSdkMcpResponse(transport.writes, 'call-sdk').error).toEqual({
    code: -32601,
    message: 'Invalid tools/call request',
  })
  expect(events).toEqual([
    { type: 'text_delta', text: 'continued after malformed sdk call' },
    { type: 'response', usage: { inputTokens: 3, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider handles assistant tool_use messages across run calls', async () => {
  const transport = new FakeClaudeTransport([
    {
      type: 'assistant',
      message: {
        content: [
          { type: 'text', text: 'before tool' },
          { type: 'tool_use', id: 'native-tool-1', name: 'mcp__main__shell_exec', input: { script: 'pwd' } },
        ],
      },
    },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'after native tool' }] } },
    { type: 'result', usage: { input_tokens: 2, output_tokens: 3 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const firstEvents = []
  for await (const event of provider.run(makeRequestWithoutTools([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    firstEvents.push(event)
  }

  expect(firstEvents).toEqual([
    { type: 'text_delta', text: 'before tool' },
    { type: 'tool_call_requested', toolUseId: 'native-tool-1', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])

  const secondEvents = []
  for await (const event of provider.run(
    makeRequestWithoutTools([
      {
        type: 'tool_result',
        toolUseId: 'native-tool-1',
        output: [{ type: 'text', text: '/tmp' }],
        isError: false,
      },
    ]),
  )) {
    secondEvents.push(event)
  }

  expect(transport.writes).toContainEqual({
    type: 'user',
    message: {
      role: 'user',
      content: [{ type: 'tool_result', tool_use_id: 'native-tool-1', is_error: false, content: '/tmp' }],
    },
  })
  expect(secondEvents).toEqual([
    { type: 'text_delta', text: 'after native tool' },
    { type: 'response', usage: { inputTokens: 2, outputTokens: 3, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider fails fast when an assistant tool_use has no matching tool_result', async () => {
  const transport = new FakeClaudeTransport([
    {
      type: 'assistant',
      message: {
        content: [{ type: 'tool_use', id: 'native-tool-1', name: 'mcp__main__shell_exec', input: { script: 'pwd' } }],
      },
    },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'after missing native tool result' }] } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const firstEvents = []
  for await (const event of provider.run(makeRequestWithoutTools([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    firstEvents.push(event)
  }
  expect(firstEvents).toEqual([
    { type: 'tool_call_requested', toolUseId: 'native-tool-1', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])

  const secondRun = async () => {
    for await (const _event of provider.run(makeRequestWithoutTools([{ type: 'user_message', content: [{ type: 'text', text: 'missing result' }] }]))) {
      // The provider should throw before reading more Claude output.
    }
  }
  await expect(secondRun()).rejects.toThrow('missing tool_result for tool_use native-tool-1')
  expect(transport.killed).toBe(true)
  expect(transport.waitCalls).toBe(1)
})

test('ClaudeCodeProvider reports malformed assistant tool_use blocks as provider errors', async () => {
  const transport = new FakeClaudeTransport([
    {
      type: 'assistant',
      message: {
        content: [{ type: 'tool_use', id: 'native-tool-1', input: { script: 'pwd' } }],
      },
    },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'after malformed tool use' }] } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const events = []
  for await (const event of provider.run(makeRequestWithoutTools([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    events.push(event)
    if (event.type === 'error') break
  }

  expect(events).toEqual([{ type: 'error', message: 'Invalid tool_use block from Claude Code', code: null }])
  expect(transport.writes.some((write) => isRecord(write) && write.type === 'control_response')).toBe(false)
})

test('ClaudeCodeProvider terminates active transport when consumers stop after a provider error event', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'error', message: 'bad output', code: 'stream' },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'unreachable' }] } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })
  const iterator = provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))[Symbol.asyncIterator]()

  await expect(iterator.next()).resolves.toEqual({
    done: false,
    value: { type: 'error', message: 'bad output', code: 'stream' },
  })
  await iterator.return?.()

  expect(transport.killed).toBe(true)
  expect(transport.waitCalls).toBe(1)
})

test('ClaudeCodeProvider fails fast when a pending control_request has no matching tool_result', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'control_request', id: 'call-1', method: 'tools/call', params: { name: 'mcp__main__shell_exec', arguments: { script: 'pwd' } } },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'after missing tool result' }] } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const firstEvents = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    firstEvents.push(event)
  }
  expect(firstEvents).toMatchObject([
    { type: 'tool_call_requested', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])
  const firstToolUseId = firstEvents[0]?.type === 'tool_call_requested' ? firstEvents[0].toolUseId : null
  expect(firstToolUseId).toMatch(/^mcp-control-/)

  const secondRun = async () => {
    for await (const _event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'missing result' }] }]))) {
      // The provider should throw before reading more Claude output.
    }
  }
  await expect(secondRun()).rejects.toThrow('missing tool_result for control_request call-1')
  expect(transport.killed).toBe(true)
  expect(transport.waitCalls).toBe(1)
  expect(transport.writes.some((write) => isRecord(write) && write.type === 'control_response' && write.id === 'call-1')).toBe(false)
})

test('ClaudeCodeProvider rejects malformed tools/call control requests without entering pending state', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'control_request', id: 'call-1', method: 'tools/call', params: { arguments: { script: 'pwd' } } },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'continued' }] } },
    { type: 'result', usage: { input_tokens: 1, output_tokens: 1 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const events = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    events.push(event)
  }

  expect(findControlResponse(transport.writes, 'call-1').response).toEqual({
    error: { message: 'Invalid tools/call request' },
  })
  expect(events).toEqual([
    { type: 'text_delta', text: 'continued' },
    { type: 'response', usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider reports nonzero CLI exits that do not produce result messages', async () => {
  const transport = new FakeClaudeTransport([], { exitCode: 1, stderr: 'auth failed' })
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })

  const events = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    events.push(event)
  }

  expect(events).toEqual([{ type: 'error', message: 'auth failed', code: null }])
})

test('ClaudeCodeProvider clears and terminates active transport when stdout iteration fails', async () => {
  const broken = new FakeClaudeTransport([], { throwNext: new Error('bad json') })
  const recovered = new FakeClaudeTransport([
    { type: 'assistant', message: { content: [{ type: 'text', text: 'recovered' }] } },
    { type: 'result', usage: { input_tokens: 1 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: sequenceFactory([broken, recovered]) })

  const firstRun = async () => {
    for await (const _event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
      // Consume until the iterator throws.
    }
  }
  await expect(firstRun()).rejects.toThrow('bad json')
  expect(broken.killed).toBe(true)
  expect(broken.waitCalls).toBe(1)

  const events = []
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'again' }] }]))) {
    events.push(event)
  }

  expect(recovered.writes.find((write) => isRecord(write) && write.type === 'user')).toMatchObject({ type: 'user' })
  expect(events).toEqual([
    { type: 'text_delta', text: 'recovered' },
    { type: 'response', usage: { inputTokens: 1, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 } },
  ])
})

test('ClaudeCodeProvider abort kills the active transport', async () => {
  const controller = new AbortController()
  const transport = new FakeClaudeTransport([], { hang: true, exitCode: null })
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })
  const iterator = provider
    .run({ ...makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]), cancel: controller.signal })
    [Symbol.asyncIterator]()

  const pending = iterator.next()
  await waitFor(() => transport.writes.length > 0)
  controller.abort()

  await expect(pending).resolves.toEqual({ done: true, value: undefined })
  expect(transport.killed).toBe(true)
  expect(transport.waitCalls).toBe(1)
})

test('ClaudeCodeProvider integrates with AgentSession and shell tools for control_request tool calls', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'control_request', id: 'init-1', method: 'initialize', params: {} },
    { type: 'control_request', id: 'list-1', method: 'tools/list', params: {} },
    {
      type: 'control_request',
      id: 'call-1',
      method: 'tools/call',
      params: { name: 'mcp__main__shell_exec', arguments: { script: 'printf demi-provider' } },
    },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'done' }] } },
    { type: 'result', usage: { input_tokens: 4, output_tokens: 2 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'claude-shell-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    harnessName: 'claude-shell-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const session = new AgentSession({ provider, model, cwd: process.cwd(), runtime })

  await session.send([{ type: 'text', text: 'run shell' }])

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'tool_call', 'text', 'response'])
  const toolBlock = session.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(toolBlock).toMatchObject({ type: 'tool_call', toolName: 'shell_exec', status: 'completed' })

  const callResponse = findControlResponse(transport.writes, 'call-1')
  expect(callResponse.response.isError).toBe(false)
  const shellResult = String(callResponse.response.content)
  expect(shellResult).toContain('status: exited')
  expect(shellResult).toContain('stdout:\ndemi-provider')
})

test('ClaudeCodeProvider keeps repeated MCP request ids distinct in AgentSession', async () => {
  const transport = new FakeClaudeTransport([
    { type: 'control_request', id: 'list-1', method: 'tools/list', params: {} },
    {
      type: 'control_request',
      id: '0',
      method: 'tools/call',
      params: { name: 'mcp__main__shell_exec', arguments: { script: 'printf first-tool' } },
    },
    {
      type: 'control_request',
      id: '0',
      method: 'tools/call',
      params: { name: 'mcp__main__shell_exec', arguments: { script: 'printf second-tool' } },
    },
    { type: 'assistant', message: { content: [{ type: 'text', text: 'done' }] } },
    { type: 'result', usage: { input_tokens: 4, output_tokens: 2 } },
  ])
  const provider = new ClaudeCodeProvider({ transportFactory: fakeFactory(transport) })
  const environment = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'claude-repeated-id-shell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    harnessName: 'claude-repeated-id-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const session = new AgentSession({ provider, model, cwd: process.cwd(), runtime })

  await session.send([{ type: 'text', text: 'run shell twice' }])

  const toolBlocks = session.transcript().blocks.filter((block) => block.type === 'tool_call')
  expect(toolBlocks).toHaveLength(2)
  expect(new Set(toolBlocks.map((block) => block.toolUseId)).size).toBe(2)
  expect(toolBlocks.map((block) => block.status)).toEqual(['completed', 'completed'])
  expect(toolBlocks.map((block) => block.input)).toEqual([
    JSON.stringify({ script: 'printf first-tool' }),
    JSON.stringify({ script: 'printf second-tool' }),
  ])

  const responses = transport.writes.filter((write): write is { type: string; id: string; response: Record<string, unknown> } => {
    return isRecord(write) && write.type === 'control_response' && write.id === '0' && isRecord(write.response)
  })
  expect(responses).toHaveLength(2)
  expect(String(responses[0].response.content)).toContain('first-tool')
  expect(String(responses[1].response.content)).toContain('second-tool')
})

function fakeFactory(transport: FakeClaudeTransport): ClaudeTransportFactory {
  return { start: async () => transport }
}

function sequenceFactory(transports: FakeClaudeTransport[]): ClaudeTransportFactory {
  let index = 0
  return {
    start: async () => {
      const transport = transports[index]
      index += 1
      if (!transport) throw new Error('No fake transport left')
      return transport
    },
  }
}

class FakeClaudeTransport implements ClaudeTransport, AsyncIterator<unknown> {
  readonly writes: unknown[] = []
  killed = false
  waitCalls = 0
  private index = 0

  constructor(
    private readonly queue: unknown[],
    private readonly options: { exitCode?: number | null; stderr?: string; hang?: boolean; throwNext?: Error } = {},
  ) {}

  async writeJson(value: unknown): Promise<void> {
    this.writes.push(value)
    if (isSdkInitializeRequest(value)) {
      this.queue.splice(this.index, 0, {
        type: 'control_response',
        response: {
          subtype: 'success',
          request_id: value.request_id,
          response: {},
        },
      })
    }
  }

  messages(): AsyncIterable<unknown> {
    return {
      [Symbol.asyncIterator]: () => this,
    }
  }

  async next(): Promise<IteratorResult<unknown>> {
    if (this.options.throwNext) throw this.options.throwNext
    if (this.index >= this.queue.length) {
      if (this.options.hang) return new Promise<IteratorResult<unknown>>(() => {})
      return { done: true, value: undefined }
    }
    const value = this.queue[this.index]
    this.index += 1
    return { done: false, value }
  }

  async kill(): Promise<void> {
    this.killed = true
  }

  async wait(): Promise<{ exitCode: number | null }> {
    this.waitCalls += 1
    return { exitCode: this.options.exitCode ?? 0 }
  }

  stderrText(): string {
    return this.options.stderr ?? ''
  }
}

function findControlResponse(
  writes: unknown[],
  id: string,
): { response: Record<string, unknown> } {
  const found = writes.find((write): write is { type: string; id: string; response: Record<string, unknown> } => {
    return isRecord(write) && write.type === 'control_response' && write.id === id && isRecord(write.response)
  })
  if (!found) throw new Error(`missing control_response ${id}`)
  return found
}

function sdkMcpRequest(outerId: string, id: string, method: string, params?: unknown): unknown {
  return {
    type: 'control_request',
    request_id: outerId,
    request: {
      subtype: 'mcp_message',
      server_name: 'main',
      message: { jsonrpc: '2.0', id, method, params },
    },
  }
}

function findSdkMcpResponse(writes: unknown[], outerId: string): { result?: unknown; error?: unknown } {
  const found = writes.find((write): write is { response: { response: { mcp_response: { result?: unknown; error?: unknown } } } } => {
    return (
      isRecord(write) &&
      write.type === 'control_response' &&
      isRecord(write.response) &&
      write.response.request_id === outerId &&
      isRecord(write.response.response) &&
      isRecord(write.response.response.mcp_response)
    )
  })
  if (!found) throw new Error(`missing SDK MCP response ${outerId}`)
  return found.response.response.mcp_response
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function isSdkInitializeRequest(value: unknown): value is { request_id: string } {
  return (
    isRecord(value) &&
    value.type === 'control_request' &&
    typeof value.request_id === 'string' &&
    isRecord(value.request) &&
    value.request.subtype === 'initialize'
  )
}

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) return
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
  throw new Error('condition was not met')
}
