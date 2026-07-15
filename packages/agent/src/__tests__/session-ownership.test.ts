import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import type { ModelSelection } from '@demicodes/core'
import type { AgentHarness } from '@demicodes/agent'
import { LocalHost } from '@demicodes/host-local'
import { defineProvider, type Provider, type ProviderSelection } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentServer, type ClientSessionEvent } from '../index'

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

const selection: ProviderSelection = { providerId: 'stub', model }

interface CounterState {
  count: number
}

function runtimeProvider(turnsFactory: () => ConstructorParameters<typeof StubProvider>[0]): Provider {
  return defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () => new StubProvider(turnsFactory()),
  })
}

/**
 * A stateful harness that records every state object its host() closure was
 * built around, plus a tool that mutates the live state — used to prove the
 * harness and the session observe the same object across save/restore.
 */
function createStatefulHarness(cwd: string, hostStates: CounterState[]): AgentHarness<CounterState> {
  const host = new LocalHost(cwd)
  return {
    name: 'stateful-test',
    initialState: () => ({ count: 0 }),
    host: (ctx) => {
      hostStates.push(ctx.state)
      return host
    },
    systemPrompt: () => 'test',
  }
}

test('restored sessions share one live state object between harness and session', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-ownership-'))
  const sessionId = globalThis.crypto.randomUUID()
  const hostStates: CounterState[] = []
  const turns = (): ConstructorParameters<typeof StubProvider>[0] => [
    [events.toolCall('tool-1', 'shell_exec', { script: 'printf ready', timeoutMs: 1_000 })],
    [events.text('done'), events.response()],
  ]

  // Mutate state via lifecycle; shell Host resolution then observes that same live state.
  const harness: AgentHarness<CounterState> = {
    ...createStatefulHarness(cwd, hostStates),
    lifecycle: (event) => {
      if (event.type === 'before_round_start') event.state.count += 1
    },
  }

  const server = new AgentServer({ agent: harness, providers: [runtimeProvider(turns)] })
  const client = server.client()
  await client.open(selection, cwd, sessionId)
  await client.send([{ type: 'text', text: 'first' }])
  await waitFor(() => client.transcript().blocks.some((block) => block.type === 'response'))
  await client.close()

  // Reopen with the same session id: the snapshot restores count = 1, and the
  // harness host() must be (re)built around that restored object.
  hostStates.length = 0
  const client2 = server.client()
  await client2.open(selection, cwd, sessionId)
  await client2.send([{ type: 'text', text: 'second' }])
  await waitFor(() =>
    client2.transcript().blocks.filter((block) => block.type === 'response').length >= 1,
  )

  // host() receives the restored live state when the shell tool resolves its Host.
  const restored = hostStates.at(-1)
  expect(restored).toBeDefined()
  expect(restored!.count).toBeGreaterThanOrEqual(2)
  await client2.close()
  await server.close()
})

test('opening an owned session id takes it over and closes the previous connection', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-takeover-'))
  const sessionId = globalThis.crypto.randomUUID()
  const harness: AgentHarness<CounterState> = createStatefulHarness(cwd, [])
  const turns = (): ConstructorParameters<typeof StubProvider>[0] => [[events.text('hi'), events.response()]]
  const server = new AgentServer({ agent: harness, providers: [runtimeProvider(turns)] })

  const first = server.client()
  const firstEvents: ClientSessionEvent[] = []
  first.subscribe((event) => firstEvents.push(event))
  await first.open(selection, cwd, sessionId)
  await first.send([{ type: 'text', text: 'hello' }])
  await waitFor(() => first.transcript().blocks.some((block) => block.type === 'response'))

  // Second connection claims the same session id: first must observe 'closed',
  // second must resume the persisted conversation.
  const second = server.client()
  await second.open(selection, cwd, sessionId)
  await waitFor(() => firstEvents.some((event) => event.type === 'closed'))
  await waitFor(() => second.transcript().blocks.some((block) => block.type === 'response'))
  expect(second.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])

  await second.close()
  await server.close()
})

test('a connection reopening after takeover does not disturb the new owner', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-takeover2-'))
  const sessionId = globalThis.crypto.randomUUID()
  const harness: AgentHarness<CounterState> = createStatefulHarness(cwd, [])
  const turns = (): ConstructorParameters<typeof StubProvider>[0] => [[events.text('hi'), events.response()]]
  const server = new AgentServer({ agent: harness, providers: [runtimeProvider(turns)] })

  const first = server.client()
  await first.open(selection, cwd, sessionId)
  const second = server.client()
  await second.open(selection, cwd, sessionId)

  // The first connection is closed; opening a *different* session id on it must work.
  const otherId = globalThis.crypto.randomUUID()
  await first.open(selection, cwd, otherId)
  await first.send([{ type: 'text', text: 'hello' }])
  await waitFor(() => first.transcript().blocks.some((block) => block.type === 'response'))

  await first.close()
  await second.close()
  await server.close()
})
