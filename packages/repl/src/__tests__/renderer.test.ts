import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import type { Block, ModelSelection, UserContentBlock } from '@demicodes/core'
import { defineProvider, type AgentProvider, type Provider, type ProviderModelList, type ProviderSelection } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentServer } from '@demicodes/agent'
import type { AbortResult, AgentHarness } from '@demicodes/agent'
import { LocalHost } from '@demicodes/host-local'
import { attachRenderer, createRenderer, handleCommand, renderEvent, resolveReplModel, runInputLoop, type ReplOutput } from '../index'

const model: ModelSelection = {
  providerId: 'claude-code',
  model: {
    id: 'claude-opus-4-8',
    name: 'Claude Opus 4.8',
    contextWindow: 200_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: { type: 'effort', effort: 'medium', summary: null },
}

test('REPL renderer prints transcript deltas, tool state, and cache usage without duplicates', () => {
  const output = new CaptureOutput()
  const renderer = createRenderer(output)
  const thinking = block({
    type: 'thinking',
    id: 'thinking-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    text: 'plan',
    signature: null,
  })
  const text = block({
    type: 'text',
    id: 'text-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    text: 'hello',
  })
  const steer = block({
    type: 'steer',
    id: 'steer-1',
    turnId: 'turn-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    content: [{ type: 'text', text: 'keep it concise' }],
  })
  const tool = block({
    type: 'tool_call',
    id: 'tool-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    toolUseId: 'tool-use-1',
    toolName: 'shell_exec',
    input: JSON.stringify({ script: 'bun test' }),
    status: 'executing',
    streamingOutput: [],
    output: [],
    metadata: null,
  })
  const toolError = block({
    type: 'tool_call',
    id: 'tool-2',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    toolUseId: 'tool-use-2',
    toolName: 'shell_exec',
    input: JSON.stringify({ script: 'cat sentinel.txt' }),
    status: 'error',
    streamingOutput: [],
    output: [{ type: 'text', text: 'Repeated identical shell_exec suppressed.\nUse a different command.' }],
    metadata: null,
  })
  const response = block({
    type: 'response',
    id: 'response-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    usage: { inputTokens: 10, outputTokens: 2, cacheReadTokens: 3, cacheWriteTokens: 4 },
  })
  const error = block({
    type: 'error',
    id: 'error-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    message: 'provider warning',
    code: null,
  })
  const abort = block({
    type: 'abort',
    id: 'abort-1',
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    isResumed: false,
  })

  renderEvent(renderer, { type: 'transcript_snapshot', blocks: [thinking, steer, text, tool, toolError, response, error, abort] })

  expect(output.text()).toContain('thinking> plan')
  expect(output.text()).toContain('steer> keep it concise')
  expect(output.text()).toContain('assistant> hello')
  expect(output.text()).toContain('tool> shell_exec executing -- bun test')
  expect(output.text()).toContain('tool> shell_exec error -- cat sentinel.txt')
  expect(output.text()).toContain('tool-error> Repeated identical shell_exec suppressed.')
  expect(output.text()).toContain('usage> in=10 out=2 cache_read=3 cache_write=4')
  expect(output.text()).toContain('error> agent provider warning')
  expect(output.text()).toContain('state> turn aborted')

  const offset = output.text().length
  renderEvent(renderer, {
    type: 'transcript_patch',
    patches: [],
    blocks: [
      { ...thinking, text: 'plan more' },
      steer,
      { ...text, text: 'hello world' },
      { ...tool, status: 'completed', output: [{ type: 'text', text: 'ok' }] },
      toolError,
      response,
      error,
      { ...abort, isResumed: true },
    ],
  })
  const delta = output.text().slice(offset)

  expect(delta).toContain(' more')
  expect(delta).toContain(' world')
  expect(delta).toContain('tool> shell_exec completed -- bun test')
  expect(delta).not.toContain('hello world')
  expect(delta).not.toContain('Repeated identical shell_exec suppressed.')
  expect(delta).not.toContain('usage>')
  expect(delta).not.toContain('error> agent')
  expect(delta).not.toContain('state> turn aborted')
})

test('REPL renderer prints phase, queue, shell output, audit, and progress frames', () => {
  const output = new CaptureOutput()
  const renderer = createRenderer(output)

  renderEvent(renderer, { type: 'phase', phase: 'running' })
  renderEvent(renderer, {
    type: 'queue',
    queue: [{ id: 'queued-1', text: 'next task', content: [{ type: 'text', text: 'next task' }] }],
  })
  renderEvent(renderer, {
    type: 'shell_output',
    shellId: 'shell-1',
    commandId: 'command-1',
    snapshot: {
      status: 'running',
      shellId: 'shell-1',
      commandId: 'command-1',
      stdout: { path: 'demi://stdout', offset: 4, delta: 'out\n', tail: 'out\n', bytes: 4, truncated: false },
      stderr: { path: 'demi://stderr', offset: 4, delta: 'err\n', tail: 'err\n', bytes: 4, truncated: false },
      output: {
        path: 'demi://output',
        offset: 8,
        text: 'out\nerr\n',
        tail: 'out\nerr\n',
        chunks: [
          { stream: 'stdout', text: 'out\n' },
          { stream: 'stderr', text: 'err\n' },
        ],
        bytes: 8,
        truncated: false,
      },
      runningMs: 10,
      idleMs: 0,
    },
  })
  renderEvent(renderer, {
    type: 'audit',
    events: [
      { kind: 'registered-command', name: 'editor', args: ['list'], exitCode: 0 },
      { kind: 'system-command', name: 'bun', args: ['test'], cwd: '/tmp/project', exitCode: 1 },
    ],
  })
  renderEvent(renderer, {
    type: 'tool_progress',
    toolUseId: 'tool-1',
    output: [{ type: 'text', text: JSON.stringify({ shellId: 'shell-1', status: 'running', reason: 'yield' }) }],
  })
  renderEvent(renderer, {
    type: 'tool_progress',
    toolUseId: 'tool-2',
    output: [{ type: 'text', text: 'plain progress' }],
  })
  renderEvent(renderer, { type: 'error', message: 'provider failed', code: 'rate_limit' })
  renderEvent(renderer, { type: 'rejected', command: 'retry', reason: 'busy' })
  renderEvent(renderer, { type: 'closed' })

  const text = output.text()
  expect(text).toContain('state> running')
  expect(text).toContain('queue> 1 pending')
  expect(text).toContain('shell[command-1] stdout> out')
  expect(text).toContain('shell[command-1] stderr> err')
  expect(text).toContain('audit> registered editor list -> 0')
  expect(text).toContain('audit> system bun test -> 1')
  expect(text).toContain('progress> shell[shell-1] running (yield)')
  expect(text).toContain('progress> plain progress')
  expect(text).toContain('error> provider failed')
  expect(text).toContain('error> retry rejected: busy')
  expect(text).toContain('state> closed')
})

test('REPL renderer summarizes every standard tool with description or fallback', () => {
  const output = new CaptureOutput()
  const renderer = createRenderer(output)
  const toolBlocks = [
    toolBlock('tool-exec', 'shell_exec', { description: 'Run project tests', script: 'bun test' }, 'executing'),
    toolBlock('tool-status', 'shell_status', { description: 'Check dev server output', commandId: 'cmd-1' }),
    toolBlock('tool-write', 'shell_write', { commandId: 'cmd-1', stdin: 'typed\n' }),
    toolBlock('tool-abort', 'shell_abort', { commandId: 'cmd-2' }),
    toolBlock('tool-yield', 'yield', { durationMs: 250 }),
    toolBlock('tool-unknown', 'unknown_tool', { value: 'raw' }),
  ]

  renderEvent(renderer, { type: 'transcript_snapshot', blocks: toolBlocks })

  const text = output.text()
  expect(text).toContain('tool> shell_exec executing -- Run project tests')
  expect(text).toContain('tool> shell_status completed -- Check dev server output')
  expect(text).toContain('tool> shell_write completed -- Send input to cmd-1')
  expect(text).toContain('tool> shell_abort completed -- Stop cmd-2')
  expect(text).toContain('tool> yield completed -- Wait 250ms')
  expect(text).toContain('tool> unknown_tool completed -- {"value":"raw"}')
})

test('REPL renderer receives AgentClient subscription events end to end', async () => {
  const provider = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () =>
      new StubProvider([
        [
          events.text('agent hello'),
          events.response({ inputTokens: 6, outputTokens: 2, cacheReadTokens: 1, cacheWriteTokens: 0 }),
        ],
      ]),
  })
  const server = new AgentServer({
    agent: testHarness,
    providers: [provider],
  })
  const client = server.client()
  const output = new CaptureOutput()
  const renderer = createRenderer(output)
  attachRenderer(client, renderer)

  const selection: ProviderSelection = { providerId: 'stub', model: { ...model, providerId: 'stub' } }

  await client.open(selection, '/tmp/demi-repl-test', globalThis.crypto.randomUUID())
  await client.send([{ type: 'text', text: 'hello' }])
  await client.close()
  await server.close()

  const text = output.text()
  expect(text).toContain('state> idle')
  expect(text).toContain('state> running')
  expect(text).toContain('assistant> agent hello')
  expect(text).toContain('usage> in=6 out=2 cache_read=1 cache_write=0')
  expect(text).toContain('state> closed')
})

test('REPL model resolver selects from provider catalog when no full model id is provided', async () => {
  const provider = catalogProvider('claude-code', () => ({
      providerId: 'claude-code',
      defaultModelId: 'claude-sonnet-4-6',
      sourceFetchedAt: '2026-06-20T00:00:00.000Z',
      stale: false,
      warnings: ['catalog warning'],
      models: [
        {
          providerId: 'claude-code',
          id: 'claude-opus-4-8',
          displayName: 'Claude Opus 4.8',
          contextWindow: 1_000_000,
          outputLimit: 128_000,
          supportsTools: true,
          supportsAttachments: true,
          supportsReasoning: true,
          supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max'],
          defaultThinkingEffort: 'medium',
          sourceFetchedAt: '2026-06-20T00:00:00.000Z',
          stale: false,
        },
        {
          providerId: 'claude-code',
          id: 'claude-sonnet-4-6',
          displayName: 'Claude Sonnet 4.6',
          contextWindow: 1_000_000,
          outputLimit: 64_000,
          supportsTools: true,
          supportsAttachments: false,
          supportsReasoning: true,
          supportedThinkingEfforts: ['low', 'medium', 'high', 'max'],
          defaultThinkingEffort: 'medium',
          sourceFetchedAt: '2026-06-20T00:00:00.000Z',
          stale: false,
        },
      ],
    }))

  const resolved = await resolveReplModel(provider, {
    provider: 'claude-code',
    cwd: '/tmp',
    modelId: null,
    thinkingEffort: 'medium',
    serviceTierId: null,
    openAIWireApi: 'responses' as const,
    transport: 'auto',
  })

  expect(resolved.selection.model).toMatchObject({
    id: 'claude-sonnet-4-6',
    name: 'Claude Sonnet 4.6',
    contextWindow: 1_000_000,
    acceptedExtensions: [],
  })
  expect(resolved.selection.model.thinking).toEqual([
    {
      type: 'effort',
      efforts: ['low', 'medium', 'high', 'max'],
      defaultEffort: null,
      summaries: ['auto', 'concise', 'detailed', 'off', 'on'],
      defaultSummary: null,
    },
  ])
  expect(resolved.selection.thinking).toEqual({ type: 'effort', effort: 'medium', summary: null })
  expect(resolved.warnings).toEqual(['catalog warning'])
})

test('REPL model resolver rejects explicit thinking efforts not advertised by catalog', async () => {
  const provider = catalogProvider('claude-code', () => ({
      providerId: 'claude-code',
      defaultModelId: 'claude-sonnet-4-6',
      sourceFetchedAt: '2026-06-20T00:00:00.000Z',
      stale: false,
      warnings: [],
      models: [
        {
          providerId: 'claude-code',
          id: 'claude-sonnet-4-6',
          displayName: 'Claude Sonnet 4.6',
          contextWindow: 1_000_000,
          outputLimit: 64_000,
          supportsTools: true,
          supportsAttachments: false,
          supportsReasoning: true,
          supportedThinkingEfforts: ['low'],
          defaultThinkingEffort: null,
          sourceFetchedAt: '2026-06-20T00:00:00.000Z',
          stale: false,
        },
      ],
    }))

  await expect(resolveReplModel(provider, {
    provider: 'claude-code',
    cwd: '/tmp',
    modelId: null,
    thinkingEffort: 'medium',
    serviceTierId: null,
    openAIWireApi: 'responses' as const,
    transport: 'auto',
  })).rejects.toThrow('does not support thinking effort "medium"')
})

test('REPL model resolver accepts provider-advertised future thinking effort ids', async () => {
  const provider = catalogProvider('claude-code', () => ({
      providerId: 'claude-code',
      defaultModelId: 'claude-opus-4-8',
      sourceFetchedAt: '2026-06-20T00:00:00.000Z',
      stale: false,
      warnings: [],
      models: [
        {
          providerId: 'claude-code',
          id: 'claude-opus-4-8',
          displayName: 'Claude Opus 4.8',
          contextWindow: 1_000_000,
          outputLimit: 128_000,
          supportsTools: true,
          supportsAttachments: true,
          supportsReasoning: true,
          supportedThinkingEfforts: ['ultra'],
          defaultThinkingEffort: null,
          sourceFetchedAt: '2026-06-20T00:00:00.000Z',
          stale: false,
        },
      ],
    }))

  const resolved = await resolveReplModel(provider, {
    provider: 'claude-code',
    cwd: '/tmp',
    modelId: null,
    thinkingEffort: 'ultra',
    serviceTierId: null,
    openAIWireApi: 'responses',
    transport: 'auto',
  })

  expect(resolved.selection.model.thinking).toMatchObject([{ type: 'effort', efforts: ['ultra'], defaultEffort: null }])
  expect(resolved.selection.thinking).toEqual({ type: 'effort', effort: 'ultra', summary: null })
})

test('REPL model resolver validates provider-advertised service tier ids', async () => {
  const provider = catalogProvider('codex', () => ({
      providerId: 'codex',
      defaultModelId: 'gpt-5.5',
      sourceFetchedAt: '2026-06-20T00:00:00.000Z',
      stale: false,
      warnings: [],
      models: [
        {
          providerId: 'codex',
          id: 'gpt-5.5',
          displayName: 'GPT-5.5',
          contextWindow: 272_000,
          outputLimit: null,
          supportsTools: true,
          supportsAttachments: true,
          supportsReasoning: true,
          supportedThinkingEfforts: ['medium'],
          defaultThinkingEffort: null,
          serviceTiers: [{ id: 'priority', label: 'Fast' }],
          defaultServiceTierId: null,
          sourceFetchedAt: '2026-06-20T00:00:00.000Z',
          stale: false,
        },
      ],
    }))

  const resolved = await resolveReplModel(provider, {
    provider: 'codex',
    cwd: '/tmp',
    modelId: null,
    thinkingEffort: null,
    serviceTierId: 'priority',
    openAIWireApi: 'responses',
    transport: 'auto',
  })

  expect(resolved.selection.serviceTierId).toBe('priority')
  await expect(resolveReplModel(provider, {
    provider: 'codex',
    cwd: '/tmp',
    modelId: null,
    thinkingEffort: null,
    serviceTierId: 'fast',
    openAIWireApi: 'responses',
    transport: 'auto',
  })).rejects.toThrow('does not support service tier "fast"')
})

test('REPL model resolver rejects aliases and does not call model catalog for explicit full ids', async () => {
  let listCalls = 0
  const provider = catalogProvider('claude-code', () => {
      listCalls += 1
      throw new Error('catalog should not be called for explicit model ids')
  })
  const baseOptions = {
    provider: 'claude-code' as const,
    cwd: '/tmp',
    thinkingEffort: null,
    serviceTierId: null,
    openAIWireApi: 'responses' as const,
    transport: 'auto' as const,
    timeoutMs: 10,
  }

  await expect(resolveReplModel(provider, { ...baseOptions, modelId: 'opus' })).rejects.toThrow('not alias "opus"')
  const resolved = await resolveReplModel(provider, { ...baseOptions, modelId: 'claude-opus-4-8' })

  expect(listCalls).toBe(0)
  expect(resolved.selection.model.id).toBe('claude-opus-4-8')
  expect(resolved.selection.model.contextWindow).toBe(0)
})

test('REPL model resolver allows Anthropic-compatible explicit model ids', async () => {
  let listCalls = 0
  const provider = catalogProvider('anthropic', () => {
    listCalls += 1
    throw new Error('catalog should not be called for explicit model ids')
  })

  const resolved = await resolveReplModel(provider, {
    provider: 'anthropic',
    cwd: '/tmp',
    modelId: 'deepseek-v4-pro',
    thinkingEffort: null,
    serviceTierId: null,
    openAIWireApi: 'responses',
    transport: 'auto',
  })

  expect(listCalls).toBe(0)
  expect(resolved.selection.model.id).toBe('deepseek-v4-pro')
  expect(resolved.selection.model.name).toBe('Anthropic API deepseek-v4-pro')
})

test('REPL commands dispatch to the agent client and validate input usage', async () => {
  const output = new CaptureOutput()
  const client = new FakeCommandClient()

  await expect(handleCommand('/help', client, output)).resolves.toBe(false)
  await expect(handleCommand('/abort', client, output)).resolves.toBe(false)
  await expect(handleCommand('/steer refine the current turn', client, output)).resolves.toBe(false)
  await expect(handleCommand('/steer', client, output)).resolves.toBe(false)
  await expect(handleCommand('/retry', client, output)).resolves.toBe(false)
  await expect(handleCommand('/resume', client, output)).resolves.toBe(false)
  await expect(handleCommand('/compact', client, output)).resolves.toBe(false)
  await expect(handleCommand('/input command-1 typed words', client, output)).resolves.toBe(false)
  await expect(handleCommand('/input', client, output)).resolves.toBe(false)
  await expect(handleCommand('/bogus', client, output)).resolves.toBe(false)
  await expect(handleCommand('/exit', client, output)).resolves.toBe(true)

  expect(client.calls).toEqual([
    ['abort'],
    ['steer', 'refine the current turn'],
    ['retry'],
    ['resume'],
    ['compact'],
    ['shellWrite', 'command-1', 'typed words\n'],
  ])
  expect(output.text()).toContain('Commands:')
  expect(output.text()).toContain('state> abort requested')
  expect(output.text()).toContain('usage: /steer <message>')
  expect(output.text()).toContain('usage: /input <commandId> <text>')
  expect(output.text()).toContain('error> unknown command: /bogus')
})

test('REPL /steer prints asynchronous steer failures', async () => {
  const output = new CaptureOutput()
  const client = new FakeCommandClient()
  client.onSteer = () => Promise.reject(new Error('no active turn'))

  await expect(handleCommand('/steer refine current turn', client, output)).resolves.toBe(false)
  await waitForMicrotasks()

  expect(client.calls).toEqual([['steer', 'refine current turn']])
  expect(output.text()).toContain('error> steer failed: no active turn')
})

test('REPL input loop sends messages asynchronously so commands remain responsive', async () => {
  const output = new CaptureOutput()
  const renderer = createRenderer(output)
  const client = new FakeLoopClient()
  const sendGate = deferred<void>()
  client.onSend = () => sendGate.promise
  const prompts = promptQueue(['', 'create a file', '/abort', '/exit'])

  await runInputLoop({ ask: prompts.ask, client, renderer, output })

  expect(prompts.consumed()).toBe(4)
  expect(client.calls).toEqual([
    ['send', 'create a file'],
    ['abort'],
  ])
  sendGate.resolve()
})

test('REPL input loop prints asynchronous send failures', async () => {
  const output = new CaptureOutput()
  const renderer = createRenderer(output)
  const client = new FakeLoopClient()
  client.onSend = () => Promise.reject(new Error('provider failed'))
  const prompts = promptQueue(['run task', '/exit'])

  await runInputLoop({ ask: prompts.ask, client, renderer, output })
  await waitForMicrotasks()

  expect(client.calls).toEqual([['send', 'run task']])
  expect(output.text()).toContain('error> send failed: provider failed')
})

function block<T extends Block>(value: T): T {
  return value
}

function toolBlock(
  id: string,
  toolName: string,
  input: Record<string, unknown>,
  status: Extract<Block, { type: 'tool_call' }>['status'] = 'completed',
): Extract<Block, { type: 'tool_call' }> {
  return block({
    type: 'tool_call',
    id,
    createdAt: '2026-06-19T00:00:00.000Z',
    model,
    toolUseId: `${id}-use`,
    toolName,
    input: JSON.stringify(input),
    status,
    streamingOutput: [],
    output: [],
    metadata: null,
  })
}

function catalogProvider(id: string, listModels: () => ProviderModelList): Provider {
  return defineProvider({
    id,
    displayName: id,
    listModels,
    createRuntime: () => new StubProvider([]),
  })
}

class CaptureOutput implements ReplOutput {
  readonly isTTY = false
  private readonly chunks: string[] = []

  write(text: string): void {
    this.chunks.push(text)
  }

  text(): string {
    return this.chunks.join('')
  }
}

class FakeCommandClient {
  readonly calls: Array<[string] | [string, string] | [string, string, string]> = []
  onSteer: () => Promise<void> = () => Promise.resolve()

  async abort(): Promise<AbortResult> {
    this.calls.push(['abort'])
    return { aborted: true, target: 'active_turn', canAbortAgain: false }
  }

  async steer(content: UserContentBlock[]): Promise<void> {
    this.calls.push(['steer', content.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join('\n')])
    return this.onSteer()
  }

  async retry(): Promise<void> {
    this.calls.push(['retry'])
  }

  async resume(): Promise<void> {
    this.calls.push(['resume'])
  }

  async compact(): Promise<void> {
    this.calls.push(['compact'])
  }

  async shellWrite(commandId: string, stdin: string): Promise<void> {
    this.calls.push(['shellWrite', commandId, stdin])
  }
}

class FakeLoopClient extends FakeCommandClient {
  onSend: () => Promise<void> = () => Promise.resolve()

  async send(content: UserContentBlock[]): Promise<void> {
    this.calls.push(['send', content.map((block) => (block.type === 'text' ? block.text : `[${block.type}]`)).join('\n')])
    return this.onSend()
  }
}

const testHarness: AgentHarness<Record<string, never>> = {
  name: 'repl-test',
  initialState: () => ({}),
  host: (ctx) => new LocalHost(ctx.cwd),
  systemPrompt: () => 'system',
  preamble: () => null,
}

function promptQueue(values: string[]): { ask: () => Promise<string>; consumed: () => number } {
  let index = 0
  return {
    ask: async () => {
      const value = values[index]
      index += 1
      if (value === undefined) throw new Error('promptQueue exhausted')
      return value
    },
    consumed: () => index,
  }
}

function waitForMicrotasks(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}
