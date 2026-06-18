import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { AgentSession, type AgentDefinition } from '@demi/base-agent'
import { BashEnvironment, createShellSessionTools, type ShellToolResult } from '@demi/shell'
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
  expect(transport.writes[0]).toMatchObject({ type: 'user' })
  expect(transport.waitCalls).toBe(1)
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

  expect(firstEvents).toEqual([
    { type: 'tool_call_requested', toolUseId: 'call-1', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])
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
        toolUseId: 'call-1',
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
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    firstEvents.push(event)
  }

  expect(firstEvents).toEqual([
    { type: 'text_delta', text: 'before tool' },
    { type: 'tool_call_requested', toolUseId: 'native-tool-1', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])

  const secondEvents = []
  for await (const event of provider.run(
    makeRequest([
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
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
    firstEvents.push(event)
  }
  expect(firstEvents).toEqual([
    { type: 'tool_call_requested', toolUseId: 'native-tool-1', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])

  const secondRun = async () => {
    for await (const _event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'missing result' }] }]))) {
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
  for await (const event of provider.run(makeRequest([{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }]))) {
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
  expect(firstEvents).toEqual([
    { type: 'tool_call_requested', toolUseId: 'call-1', toolName: 'shell_exec', input: { script: 'pwd' } },
  ])

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

  expect(recovered.writes[0]).toMatchObject({ type: 'user' })
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
    sessionIdFactory: () => 'claude-shell-session',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const definition: AgentDefinition<Record<string, never>> = {
    name: 'claude-shell-test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => createShellSessionTools(environment),
  }
  const session = new AgentSession({ provider, model, cwd: process.cwd(), definition })

  await session.send([{ type: 'text', text: 'run shell' }])

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'tool_call', 'text', 'response'])
  const toolBlock = session.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(toolBlock).toMatchObject({ type: 'tool_call', toolName: 'shell_exec', status: 'completed' })

  const callResponse = findControlResponse(transport.writes, 'call-1')
  expect(callResponse.response.isError).toBe(false)
  const shellResult = JSON.parse(String(callResponse.response.content)) as ShellToolResult
  expect(shellResult.status).toBe('exited')
  expect(shellResult.output.stdoutDelta).toBe('demi-provider')
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) return
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
  throw new Error('condition was not met')
}
