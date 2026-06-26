import { access, mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { deferred } from '@demi/utils'
import type { ModelSelection } from '@demi/core'
import type { AgentHarness, AgentSessionSnapshot } from '@demi/agent'
import type { BashEnvironmentOptions } from '@demi/shell'
import { LocalHost } from '@demi/host-local'
import {
  defineProvider,
  type AgentProvider,
  type InferenceRequest,
  type InferenceSteer,
  type Provider,
  type ProviderEvent,
  type ProviderRun,
  type ProviderSelection,
} from '@demi/provider'
import { StubProvider, createProviderRun, events } from '@demi/provider/testing'
import {
  AgentClient,
  AgentServer,
  type ClientSessionEvent,
} from '../index'

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

test('AgentClient.open and send run through InProcessTransport and emit transcript/phase frames', async () => {
  const turns: ConstructorParameters<typeof StubProvider>[0] = [
    [
      events.text('hello'),
      events.response({ inputTokens: 11, outputTokens: 7, cacheReadTokens: 5, cacheWriteTokens: 3 }),
    ],
  ]
  const { client } = createAgentClientHarness({
    providerTurns: turns,
  })
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerConfig(turns), '/workspace')
  await client.send([{ type: 'text', text: 'hi' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  expect(client.transcript().blocks[2]).toMatchObject({
    type: 'response',
    usage: { inputTokens: 11, outputTokens: 7, cacheReadTokens: 5, cacheWriteTokens: 3 },
  })
  const responsePatch = seen
    .filter((event) => event.type === 'transcript_patch')
    .flatMap((event) => (event.type === 'transcript_patch' ? event.patches : []))
    .find((patch) => patch.op === 'add' && patch.value.type === 'response')
  expect(responsePatch).toMatchObject({
    op: 'add',
    value: {
      type: 'response',
      usage: { inputTokens: 11, outputTokens: 7, cacheReadTokens: 5, cacheWriteTokens: 3 },
    },
  })
  expect(seen.map((event) => event.type)).toContain('transcript_snapshot')
  expect(seen.map((event) => event.type)).toContain('transcript_patch')
  expect(seen).toContainEqual({ type: 'phase', phase: 'idle' })
})

test('AgentClient clears its local transcript view when the session is closed', async () => {
  const { client } = createAgentClientHarness({
    providerTurns: [[events.text('hello'), events.response()]],
  })
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerConfig([[events.text('hello'), events.response()]]), '/workspace')
  await client.send([{ type: 'text', text: 'hi' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  expect(client.transcript().blocks.length).toBeGreaterThan(0)
  await client.close()

  expect(seen.some((event) => event.type === 'closed')).toBe(true)
  expect(client.transcript().blocks).toEqual([])
})

test('AgentServer persists session snapshots through Host.store', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-agent-server-store-'))
  const host = new LocalHost(root, { storeRoot: join(root, '.host-store') })
  const harness: AgentHarness<Record<string, never>> = {
    name: 'stored-session',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'system',
  }
  const turns: ConstructorParameters<typeof StubProvider>[0] = [[events.text('stored'), events.response()]]
  const { client } = createAgentClientHarness({ harness, providerTurns: turns })

  await client.open(providerConfig(turns), root)
  await client.send([{ type: 'text', text: 'persist me' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  const keys = await host.store.list('agent-sessions')
  expect(keys).toHaveLength(1)
  expect(keys[0]).toEndWith('/snapshot.json')
  const snapshot = await host.store.readJson<AgentSessionSnapshot<Record<string, never>>>(keys[0])
  expect(snapshot).toMatchObject({
    cwd: root,
    harnessName: 'stored-session',
    phase: 'running',
  })
  expect(snapshot?.transcript.blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

test('AgentServer forwards provider error codes once and preserves the transcript error block', async () => {
  const turns: ConstructorParameters<typeof StubProvider>[0] = [[events.error('auth failed', 'auth')]]
  const { client } = createAgentClientHarness({ providerTurns: turns })
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerConfig(turns), '/workspace')
  await expect(client.send([{ type: 'text', text: 'hi' }])).rejects.toThrow('auth failed')
  await waitFor(() => seen.some((event) => event.type === 'error'))
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'error'))
  await delay(5)

  const errors = seen.filter((event) => event.type === 'error')
  expect(errors).toEqual([{ type: 'error', message: 'auth failed', code: 'auth' }])
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'error'])
  expect(client.transcript().blocks[1]).toMatchObject({
    type: 'error',
    message: 'auth failed',
    code: 'auth',
  })
})

test('AgentServer maps shell tool progress into shell_output and audit frames', async () => {
  const { client } = createAgentClientHarness({
    shell: {
      initialEnv: { PATH: process.env.PATH ?? '' },
      shellIdFactory: () => 'agent-shell',
    },
    providerTurns: [
      [events.toolCall('tool-1', 'shell_exec', { script: 'sh -c "printf hi"', timeoutMs: 1_000 })],
      [events.text('done'), events.response()],
    ],
  })
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerConfig([
      [events.toolCall('tool-1', 'shell_exec', { script: 'sh -c "printf hi"', timeoutMs: 1_000 })],
      [events.text('done'), events.response()],
    ]),
    process.cwd(),
  )
  await client.send([{ type: 'text', text: 'run shell' }])
  await waitFor(() => seen.some((event) => event.type === 'shell_output'))

  const shellOutput = seen.find((event) => event.type === 'shell_output')
  expect(shellOutput).toMatchObject({
    type: 'shell_output',
    shellId: 'agent-shell',
    commandId: expect.any(String),
    snapshot: { stdout: { delta: 'hi' } },
  })
  expect(seen.some((event) => event.type === 'audit')).toBe(true)
})

test('AgentServer bridges shell_write frames to the active shell command', async () => {
  const { client } = createAgentClientHarness({
    shell: {
      initialEnv: { PATH: process.env.PATH ?? '' },
      shellIdFactory: () => 'agent-input-shell',
      commandIdFactory: () => 'agent-input-command',
    },
    providerTurns: [
      [
        events.toolCall('tool-1', 'shell_exec', {
          script: 'sh -c \'IFS= read -r line; printf %s "$line"\'',
          timeoutMs: 1,
        }),
      ],
      [events.text('waiting'), events.response()],
    ],
  })
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerConfig([
      [
        events.toolCall('tool-1', 'shell_exec', {
          script: 'sh -c \'IFS= read -r line; printf %s "$line"\'',
          timeoutMs: 1,
        }),
      ],
      [events.text('waiting'), events.response()],
    ]),
    process.cwd(),
  )
  await client.send([{ type: 'text', text: 'start process' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  seen.length = 0
  await client.shellWrite('agent-input-command', 'typed\n')
  await waitFor(() => seen.some((event) => event.type === 'shell_write_result' && event.commandId === 'agent-input-command'))

  expect(seen).toContainEqual({
    type: 'shell_output',
    shellId: 'agent-input-shell',
    commandId: 'agent-input-command',
    snapshot: {
      status: 'running',
      shellId: 'agent-input-shell',
      commandId: 'agent-input-command',
      stdout: expect.objectContaining({ delta: '', truncated: false }),
      stderr: expect.objectContaining({ delta: '', tail: '', bytes: 0, truncated: false }),
      output: expect.objectContaining({ chunks: expect.any(Array) }),
      runningMs: expect.any(Number),
      idleMs: expect.any(Number),
    },
  })
  expect(seen).not.toContainEqual({
    type: 'tool_progress',
    toolUseId: 'agent-shell-write:agent-input-command',
    output: expect.any(Array),
  })
})

test('AgentClient.shellWrite waits for shell_write_result and rejects when no session is open', async () => {
  const unopened = createAgentClientHarness({ providerTurns: [] })
  await expect(unopened.client.shellWrite('missing-command', 'stdin')).rejects.toThrow('No session is open')

  const seen: ClientSessionEvent[] = []
  const { client } = createAgentClientHarness({
    shell: {
      initialEnv: { PATH: process.env.PATH ?? '' },
      shellIdFactory: () => 'agent-delayed-input-shell',
      commandIdFactory: () => 'agent-delayed-input-command',
    },
    providerTurns: [
      [
        events.toolCall('tool-1', 'shell_exec', {
          script: 'sh -c \'IFS= read -r line; sleep 0.05; printf %s "$line"\'',
          timeoutMs: 1,
        }),
      ],
      [events.text('waiting'), events.response()],
    ],
  })
  client.subscribe((event) => seen.push(event))
  const turns: ConstructorParameters<typeof StubProvider>[0] = [
    [
      events.toolCall('tool-1', 'shell_exec', {
        script: 'sh -c \'IFS= read -r line; sleep 0.05; printf %s "$line"\'',
        timeoutMs: 1,
      }),
    ],
    [events.text('waiting'), events.response()],
  ]
  await client.open(providerConfig(turns), process.cwd())
  await client.send([{ type: 'text', text: 'start process' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  let settled = false
  const writing = client.shellWrite('agent-delayed-input-command', 'accepted\n').then(() => {
    settled = true
  })
  await delay(5)
  expect(settled).toBe(true)

  await writing
  expect(settled).toBe(true)
  expect(seen.some((event) => event.type === 'shell_write_result' && event.commandId === 'agent-delayed-input-command')).toBe(true)
  expect(seen).not.toContainEqual({
    type: 'tool_progress',
    toolUseId: 'agent-shell-write:agent-delayed-input-command',
    output: expect.any(Array),
  })
})

test('AgentServer emits transcript patches with removals on retry', async () => {
  const { client } = createAgentClientHarness({
    providerTurns: [
      [events.text('old'), events.response()],
      (request: InferenceRequest) => {
        expect(request.items.map((item) => item.type)).toEqual(['user_message'])
        return [events.text('new'), events.response()]
      },
    ],
  })
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerConfig([
      [events.text('old'), events.response()],
      (request: InferenceRequest) => {
        expect(request.items.map((item) => item.type)).toEqual(['user_message'])
        return [events.text('new'), events.response()]
      },
    ]),
    '/workspace',
  )
  await client.send([{ type: 'text', text: 'question' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  seen.length = 0
  await client.retry()
  await waitFor(() => {
    const textBlocks = client.transcript().blocks.filter((block) => block.type === 'text')
    return textBlocks.some((block) => block.type === 'text' && block.text === 'new')
  })

  const patches = seen
    .filter((event) => event.type === 'transcript_patch')
    .flatMap((event) => (event.type === 'transcript_patch' ? event.patches : []))
  expect(patches.some((patch) => patch.op === 'remove')).toBe(true)
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

test('AgentServer queues send frames while the session is busy and drains them in order', async () => {
  const gate = deferred<void>()
  const provider = new DelayedProvider(gate.promise)
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('delayed', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('delayed'), '/workspace')
  let firstSettled = false
  const firstSend = client.send([{ type: 'text', text: 'first' }]).then(() => {
    firstSettled = true
  })
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))
  expect(firstSettled).toBe(false)

  let secondSettled = false
  const secondSend = client.send([{ type: 'text', text: 'second' }]).then(() => {
    secondSettled = true
  })
  await waitFor(() => seen.some((event) => event.type === 'queue' && event.queue.some((message) => message.text === 'second')))

  expect(seen.some((event) => event.type === 'rejected' && event.command === 'send')).toBe(false)
  expect(secondSettled).toBe(false)
  gate.resolve()
  await firstSend
  await secondSend
  expect(firstSettled).toBe(true)
  expect(secondSettled).toBe(true)
  await waitFor(() => provider.calls === 2)
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response', 'user', 'text', 'response'])
  const userTexts = client
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .map((block) => (block.type === 'user' && block.content[0]?.type === 'text' ? block.content[0].text : ''))
  expect(userTexts).toEqual(['first', 'second'])
  expect(seen.some((event) => event.type === 'queue' && event.queue.length === 0)).toBe(true)
})

test('AgentClient.steer resolves correlated accepted acks and receives transcript patches without queueing', async () => {
  const provider = new ServerGateProvider({ supportsSteer: true })
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('server-steerable', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('server-steerable'), '/workspace')
  const sending = client.send([{ type: 'text', text: 'start' }])
  await provider.waitForRun(0)
  seen.length = 0

  await Promise.all([
    client.steer([{ type: 'text', text: 'first steer' }]),
    client.steer([{ type: 'text', text: 'second steer' }]),
  ])

  const steerResults = seen.filter((event) => event.type === 'steer_result')
  expect(steerResults).toHaveLength(2)
  expect(steerResults.every((event) => event.type === 'steer_result' && event.status === 'accepted')).toBe(true)
  expect(new Set(steerResults.map((event) => (event.type === 'steer_result' ? event.steerId : ''))).size).toBe(2)
  expect(provider.steers.map((steer) => steer.content)).toEqual([
    [{ type: 'text', text: 'first steer' }],
    [{ type: 'text', text: 'second steer' }],
  ])
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'steer', 'steer'])
  expect(seen.some((event) => event.type === 'queue')).toBe(false)

  provider.release(0)
  await sending
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'steer', 'steer', 'text', 'response'])
})

test('AgentClient.steer rejects when no session is open and does not create transcript state', async () => {
  const { client } = createAgentClientHarness({ providerTurns: [] })
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await expect(client.steer([{ type: 'text', text: 'orphan steer' }])).rejects.toThrow('No session is open')

  expect(seen).toContainEqual({
    type: 'steer_result',
    steerId: expect.any(String),
    status: 'rejected',
    reason: 'No session is open on this connection',
  })
  expect(client.transcript().blocks).toEqual([])
})

test('AgentClient.steer accepts active provider without native steer and materializes at the continuation boundary', async () => {
  const provider = new ServerGateProvider({ supportsSteer: false })
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('server-no-native-steer', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('server-no-native-steer'), '/workspace')
  const sending = client.send([{ type: 'text', text: 'start' }])
  await provider.waitForRun(0)
  seen.length = 0

  await client.steer([{ type: 'text', text: 'same turn guidance' }])

  expect(seen).toContainEqual({
    type: 'steer_result',
    steerId: expect.any(String),
    status: 'accepted',
  })
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user'])
  expect(seen.some((event) => event.type === 'queue')).toBe(false)
  expect(
    seen
      .filter((event) => event.type === 'transcript_patch')
      .flatMap((event) => (event.type === 'transcript_patch' ? event.patches : []))
      .some((patch) => patch.op === 'add' && patch.value.type === 'steer'),
  ).toBe(false)

  provider.release(0)
  await provider.waitForRun(1)
  expect(
    seen
      .filter((event) => event.type === 'transcript_patch')
      .flatMap((event) => (event.type === 'transcript_patch' ? event.patches : []))
      .some((patch) => patch.op === 'add' && patch.value.type === 'steer'),
  ).toBe(true)
  expect(provider.requests[1]?.turnId).toBe(provider.requests[0]?.turnId)
  expect(provider.requests[1]?.items.map((item) => item.type)).toEqual(['user_message', 'assistant_text', 'user_steer'])
  expect(provider.requests[1]?.items[2]).toEqual({
    type: 'user_steer',
    turnId: provider.requests[0]?.turnId,
    content: [{ type: 'text', text: 'same turn guidance' }],
  })

  provider.release(1)
  await sending
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response', 'steer', 'text', 'response'])
})

test('AgentClient.cancelPendingSteer removes an accepted steer before transcript materialization', async () => {
  const provider = new ServerGateProvider({ supportsSteer: false })
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('server-cancel-pending-steer', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('server-cancel-pending-steer'), '/workspace')
  const sending = client.send([{ type: 'text', text: 'start' }])
  await provider.waitForRun(0)
  seen.length = 0

  await client.steer([{ type: 'text', text: 'delete before materialized' }], { steerId: 'steer-cancel-frame' })
  client.cancelPendingSteer('steer-cancel-frame')

  provider.release(0)
  await sending

  expect(provider.requests).toHaveLength(1)
  expect(seen).toContainEqual({
    type: 'steer_result',
    steerId: 'steer-cancel-frame',
    status: 'accepted',
  })
  expect(
    seen
      .filter((event) => event.type === 'transcript_patch')
      .flatMap((event) => (event.type === 'transcript_patch' ? event.patches : []))
      .some((patch) => patch.op === 'add' && patch.value.type === 'steer'),
  ).toBe(false)
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

test('AgentClient.cancelPendingSteer is silent without an open session', async () => {
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  client.cancelPendingSteer('missing-steer')
  await Promise.resolve()
  await Promise.resolve()

  expect(seen).toEqual([])
  await server.close()
})

test('AgentServer rejects retry, resume, and compact frames while the session is busy', async () => {
  const gate = deferred<void>()
  const provider = new DelayedProvider(gate.promise)
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('delayed-rejects', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('delayed-rejects'), '/workspace')
  const sending = client.send([{ type: 'text', text: 'first' }])
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))

  await expect(client.retry()).rejects.toThrow('Session is busy (running)')
  await expect(client.resume()).rejects.toThrow('Session is busy (running)')
  await expect(client.compact()).rejects.toThrow('Session is busy (running)')

  expect(
    seen.filter((event) => event.type === 'rejected').map((event) => (event.type === 'rejected' ? event.command : '')),
  ).toEqual(['retry', 'resume', 'compact'])

  gate.resolve(undefined)
  await sending

  expect(provider.calls).toBe(1)
  expect(client.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

test('AgentClient resolves each queued send promise on its own phase cycle', async () => {
  const gates = [deferred<void>(), deferred<void>(), deferred<void>()]
  const provider = new SequencedDelayedProvider(gates.map((gate) => gate.promise))
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('sequenced-delayed', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('sequenced-delayed'), '/workspace')
  const settlements: string[] = []
  const firstSend = client.send([{ type: 'text', text: 'first' }]).then(() => {
    settlements.push('first')
  })
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))

  const secondSend = client.send([{ type: 'text', text: 'second' }]).then(() => {
    settlements.push('second')
  })
  const thirdSend = client.send([{ type: 'text', text: 'third' }]).then(() => {
    settlements.push('third')
  })
  await waitFor(() =>
    seen.some(
      (event) =>
        event.type === 'queue' &&
        event.queue.some((message) => message.text === 'second') &&
        event.queue.some((message) => message.text === 'third'),
    ),
  )
  await delay(5)
  expect(settlements).toEqual([])

  gates[0].resolve(undefined)
  await firstSend
  await waitFor(() => provider.calls === 2)
  await delay(5)
  expect(settlements).toEqual(['first'])

  gates[1].resolve(undefined)
  await secondSend
  await waitFor(() => provider.calls === 3)
  await delay(5)
  expect(settlements).toEqual(['first', 'second'])

  gates[2].resolve(undefined)
  await thirdSend
  expect(settlements).toEqual(['first', 'second', 'third'])
})

test('AgentClient.dequeueMessage resolves the removed queued send without running it', async () => {
  const gates = [deferred<void>(), deferred<void>()]
  const provider = new SequencedDelayedProvider(gates.map((gate) => gate.promise))
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('sequenced-delayed', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('sequenced-delayed'), '/workspace')
  const firstSend = client.send([{ type: 'text', text: 'first' }])
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))

  let secondSettled = false
  const secondSend = client.send([{ type: 'text', text: 'second' }]).then(() => {
    secondSettled = true
  })
  await waitFor(() => queuedMessageId(seen, 'second') !== null)
  const secondId = queuedMessageId(seen, 'second')!

  client.dequeueMessage(secondId)
  await secondSend
  expect(secondSettled).toBe(true)
  await waitFor(() => seen.some((event) => event.type === 'queue' && event.queue.length === 0))

  gates[0].resolve(undefined)
  await firstSend
  await delay(5)

  expect(provider.calls).toBe(1)
  const userTexts = client
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .map((block) => (block.type === 'user' && block.content[0]?.type === 'text' ? block.content[0].text : ''))
  expect(userTexts).toEqual(['first'])
})

test('AgentClient.sendQueuedMessage moves a queued send to the next phase cycle', async () => {
  const gates = [deferred<void>(), deferred<void>(), deferred<void>()]
  const provider = new SequencedDelayedProvider(gates.map((gate) => gate.promise))
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('sequenced-delayed', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('sequenced-delayed'), '/workspace')
  const settlements: string[] = []
  const firstSend = client.send([{ type: 'text', text: 'first' }]).then(() => {
    settlements.push('first')
  })
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))

  const secondSend = client.send([{ type: 'text', text: 'second' }]).then(() => {
    settlements.push('second')
  })
  const thirdSend = client.send([{ type: 'text', text: 'third' }]).then(() => {
    settlements.push('third')
  })
  await waitFor(() => queuedMessageId(seen, 'second') !== null && queuedMessageId(seen, 'third') !== null)
  const thirdId = queuedMessageId(seen, 'third')!

  client.sendQueuedMessage(thirdId)
  await waitFor(() => latestQueueTexts(seen).join(',') === 'third,second')

  gates[0].resolve(undefined)
  await firstSend
  await waitFor(() => provider.calls === 2)
  expect(settlements).toEqual(['first'])

  gates[1].resolve(undefined)
  await thirdSend
  await waitFor(() => provider.calls === 3)
  expect(settlements).toEqual(['first', 'third'])

  gates[2].resolve(undefined)
  await secondSend
  expect(settlements).toEqual(['first', 'third', 'second'])

  const userTexts = client
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .map((block) => (block.type === 'user' && block.content[0]?.type === 'text' ? block.content[0].text : ''))
  expect(userTexts).toEqual(['first', 'third', 'second'])
})

test('AgentClient.steerQueuedMessage converts a queued send into an active steer', async () => {
  const gates = [deferred<void>(), deferred<void>()]
  const provider = new SequencedDelayedProvider(gates.map((gate) => gate.promise))
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('sequenced-delayed', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('sequenced-delayed'), '/workspace')
  const settlements: string[] = []
  const firstSend = client.send([{ type: 'text', text: 'first' }]).then(() => {
    settlements.push('first')
  })
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))

  const secondSend = client.send([{ type: 'text', text: 'second' }]).then(() => {
    settlements.push('second')
  })
  await waitFor(() => queuedMessageId(seen, 'second') !== null)
  const secondId = queuedMessageId(seen, 'second')!

  await client.steerQueuedMessage(secondId, { steerId: 'steer-queued' })
  await secondSend
  expect(settlements).toEqual(['second'])
  expect(latestQueueTexts(seen)).toEqual([])

  gates[0].resolve(undefined)
  await waitFor(() => provider.calls === 2)
  expect(provider.requests[1]!.items.map((item) => item.type)).toEqual([
    'user_message',
    'assistant_text',
    'user_steer',
  ])
  expect(provider.requests[1]!.items[2]).toEqual({
    type: 'user_steer',
    turnId: expect.any(String),
    content: [{ type: 'text', text: 'second' }],
  })

  gates[1].resolve(undefined)
  await firstSend
  expect(settlements).toEqual(['second', 'first'])

  const blockTypes = client.transcript().blocks.map((block) => block.type)
  expect(blockTypes).toEqual(['user', 'text', 'response', 'steer', 'text', 'response'])
})

test('AgentClient.clearMessageQueue resolves queued sends without canceling the active send', async () => {
  const gates = [deferred<void>(), deferred<void>(), deferred<void>()]
  const provider = new SequencedDelayedProvider(gates.map((gate) => gate.promise))
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('sequenced-delayed', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('sequenced-delayed'), '/workspace')
  const firstSend = client.send([{ type: 'text', text: 'first' }])
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))

  let secondSettled = false
  let thirdSettled = false
  const secondSend = client.send([{ type: 'text', text: 'second' }]).then(() => {
    secondSettled = true
  })
  const thirdSend = client.send([{ type: 'text', text: 'third' }]).then(() => {
    thirdSettled = true
  })
  await waitFor(() => queuedMessageId(seen, 'second') !== null && queuedMessageId(seen, 'third') !== null)

  client.clearMessageQueue()
  await Promise.all([secondSend, thirdSend])
  expect(secondSettled).toBe(true)
  expect(thirdSettled).toBe(true)
  await waitFor(() => seen.some((event) => event.type === 'queue' && event.queue.length === 0))

  gates[0].resolve(undefined)
  await firstSend
  await delay(5)

  expect(provider.calls).toBe(1)
  const userTexts = client
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .map((block) => (block.type === 'user' && block.content[0]?.type === 'text' ? block.content[0].text : ''))
  expect(userTexts).toEqual(['first'])
})

test('AgentClient rejects only the active action when queued sends continue after an error', async () => {
  const errorGate = deferred<void>()
  const successGate = deferred<void>()
  const provider = new ErrorThenDelayedProvider(errorGate.promise, successGate.promise)
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('error-then-delayed', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('error-then-delayed'), '/workspace')
  let firstError = ''
  const firstSend = client.send([{ type: 'text', text: 'first' }]).catch((error) => {
    firstError = error instanceof Error ? error.message : String(error)
  })
  await waitFor(() => seen.some((event) => event.type === 'phase' && event.phase === 'running'))

  let secondSettled = false
  const secondSend = client.send([{ type: 'text', text: 'second' }]).then(() => {
    secondSettled = true
  })
  await waitFor(() => seen.some((event) => event.type === 'queue' && event.queue.some((message) => message.text === 'second')))

  errorGate.resolve(undefined)
  await firstSend
  expect(firstError).toBe('first failed')
  await waitFor(() => provider.calls === 2)
  await delay(5)
  expect(secondSettled).toBe(false)

  successGate.resolve(undefined)
  await secondSend
  expect(secondSettled).toBe(true)
})

test('AgentClient.abort returns false while idle and true after aborting active work', async () => {
  const provider = new AbortAwareProvider()
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('abort-aware', provider)],
  })
  const client = server.client()

  await client.open(providerSelection('abort-aware'), '/workspace')
  expect(await client.abort()).toMatchObject({ aborted: false })

  const sending = client.send([{ type: 'text', text: 'start' }])
  await provider.started.promise
  await expect(client.abort()).resolves.toMatchObject({ aborted: true })
  await provider.aborted.promise
  await sending
})

test('AgentServer aborts the active session when a close frame is received', async () => {
  const provider = new AbortAwareProvider()
  const server = new AgentServer({
    agent: createTextHarness(),
    providers: [runtimeProvider('abort-aware', provider)],
  })
  const client = server.client()
  const seen: ClientSessionEvent[] = []
  client.subscribe((event) => seen.push(event))

  await client.open(providerSelection('abort-aware'), '/workspace')
  const sending = client.send([{ type: 'text', text: 'start' }])
  await provider.started.promise

  await client.close()
  await provider.aborted.promise
  await sending

  expect(provider.calls).toBe(1)
  expect(seen.some((event) => event.type === 'closed')).toBe(true)
})

test('AgentServer disposes shell resources when a close frame is received', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-agent-dispose-'))
  const leakedPath = join(root, 'agent-leaked.txt')
  const { client } = createAgentClientHarness({
    shell: {
      initialEnv: { PATH: process.env.PATH ?? '' },
      shellIdFactory: () => 'agent-dispose-shell',
    },
    providerTurns: [
      [
        events.toolCall('tool-1', 'shell_exec', {
          script: 'sh -c "sleep 0.2; printf leaked > agent-leaked.txt"',
          timeoutMs: 1,
        }),
      ],
      [events.text('waiting'), events.response()],
    ],
  })

  await client.open(providerConfig([
      [
        events.toolCall('tool-1', 'shell_exec', {
          script: 'sh -c "sleep 0.2; printf leaked > agent-leaked.txt"',
          timeoutMs: 1,
        }),
      ],
      [events.text('waiting'), events.response()],
    ]),
    root,
  )
  await client.send([{ type: 'text', text: 'start shell' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))

  await client.close()

  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

test('AgentServer.close disposes harness resources directly', async () => {
  let disposed = false
  const harness: AgentHarness<Record<string, never>> = {
    name: 'direct-close',
    initialState: () => ({}),
    host: (ctx) => new LocalHost(ctx.cwd),
    systemPrompt: () => 'system',
    dispose: () => {
      disposed = true
    },
  }
  const { client, server } = createAgentClientHarness({
    harness,
    providerTurns: [],
  })

  await client.open(providerConfig([]), '/workspace')
  await server.close()

  expect(disposed).toBe(true)
})

function createAgentClientHarness(options: {
  harness?: AgentHarness<unknown>
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  providerTurns: ConstructorParameters<typeof StubProvider>[0]
}): { client: AgentClient; server: AgentServer } {
  const server = new AgentServer({
    agent: options.harness ?? createTextHarness(),
    providers: [runtimeProvider('stub', () => new StubProvider(options.providerTurns))],
    shell: options.shell,
  })
  const client = server.client()
  return { client, server }
}

class DelayedProvider implements AgentProvider {
  calls = 0

  constructor(private readonly release: Promise<void>) {}

  async *run(): AsyncIterable<ProviderEvent> {
    this.calls += 1
    await this.release
    yield events.text('done')
    yield events.response()
  }
}

class ServerGateProvider implements AgentProvider {
  calls = 0
  readonly requests: InferenceRequest[] = []
  readonly steers: InferenceSteer[] = []
  private readonly gates: Array<{ promise: Promise<void>; resolve: (value: void) => void }> = []
  private readonly started = new Map<number, { promise: Promise<void>; resolve: (value: void) => void }>()

  constructor(private readonly options: { supportsSteer: boolean }) {}

  run(request: InferenceRequest): ProviderRun {
    const index = this.calls
    this.calls += 1
    this.requests.push(request)
    const gate = deferred<void>()
    this.gates[index] = gate
    const output = async function* (): AsyncIterable<ProviderEvent> {
      await gate.promise
      yield events.text('done')
      yield events.response()
    }
    this.started.get(index)?.resolve(undefined)
    return createProviderRun(
      output(),
      this.options.supportsSteer
        ? {
            steer: (input) => {
              this.steers.push(input)
            },
          }
        : {},
    )
  }

  waitForRun(index: number): Promise<void> {
    if (this.requests.length > index) return Promise.resolve()
    const existing = this.started.get(index)
    if (existing) return existing.promise
    const next = deferred<void>()
    this.started.set(index, next)
    return next.promise
  }

  release(index: number): void {
    this.gates[index].resolve(undefined)
  }
}

class SequencedDelayedProvider implements AgentProvider {
  calls = 0
  readonly requests: InferenceRequest[] = []

  constructor(private readonly releases: Promise<void>[]) {}

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    this.requests.push(request)
    const call = this.calls
    this.calls += 1
    await (this.releases[call] ?? Promise.resolve())
    yield events.text(`done ${call + 1}`)
    yield events.response()
  }
}

class ErrorThenDelayedProvider implements AgentProvider {
  calls = 0

  constructor(
    private readonly errorRelease: Promise<void>,
    private readonly successRelease: Promise<void>,
  ) {}

  async *run(): AsyncIterable<ProviderEvent> {
    const call = this.calls
    this.calls += 1
    if (call === 0) {
      await this.errorRelease
      yield events.error('first failed', 'test')
      return
    }
    await this.successRelease
    yield events.text('done after error')
    yield events.response()
  }
}

class AbortAwareProvider implements AgentProvider {
  calls = 0
  readonly started = deferred<void>()
  readonly aborted = deferred<void>()

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    this.calls += 1
    request.cancel.addEventListener('abort', () => this.aborted.resolve(), { once: true })
    this.started.resolve()
    await new Promise(() => {})
  }
}

function createTextHarness(): AgentHarness<Record<string, never>> {
  return {
    name: 'test',
    initialState: () => ({}),
    host: (ctx) => new LocalHost(ctx.cwd),
    systemPrompt: () => 'system',
  }
}

function providerConfig(_turns: ConstructorParameters<typeof StubProvider>[0]): ProviderSelection {
  return providerSelection('stub')
}

function providerSelection(providerId: string): ProviderSelection {
  return {
    providerId,
    model: { ...model, providerId },
  }
}

function runtimeProvider(id: string, provider: AgentProvider | (() => AgentProvider)): Provider {
  return defineProvider({
    id,
    displayName: id,
    createRuntime: () => (typeof provider === 'function' ? provider() : provider),
  })
}

function queuedMessageId(events: ClientSessionEvent[], text: string): string | null {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index]
    if (event?.type !== 'queue') continue
    const message = event.queue.find((candidate) => candidate.text === text)
    if (message) return message.id
  }
  return null
}

function latestQueueTexts(events: ClientSessionEvent[]): string[] {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index]
    if (event?.type === 'queue') return event.queue.map((message) => message.text)
  }
  return []
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > 1_000) throw new Error('Timed out waiting for predicate')
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
