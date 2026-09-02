import { mkdtemp, mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { hostlessShellFactory, probeCommand } from '@demicodes/command-loader/testing'
import type { ModelSelection } from '@demicodes/core'
import { LocalHost } from '@demicodes/shell/node'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentServer, type AgentHarness, type ClientSessionEvent } from '../index'
import type { Command } from '@demicodes/shell'

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
const selection = { providerId: 'stub', model }

async function createHosts() {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-routing-'))
  const base = join(root, 'base')
  const alice = join(root, 'alice')
  const bob = join(root, 'bob')
  await Promise.all([base, alice, bob].map((path) => mkdir(path)))
  return {
    base: new LocalHost(base),
    alice: new LocalHost(alice),
    bob: new LocalHost(bob),
    paths: { alice, bob },
  }
}

function routedHarness(hosts: Awaited<ReturnType<typeof createHosts>>): AgentHarness<Record<string, never>> {
  return {
    name: 'host-routing-test',
    initialState: () => ({}),
    commands: () => [probeCommand()],
    host: (ctx) => {
      if (!('metadata' in ctx)) return hosts.base
      const identity = ctx.metadata?.identity
      if (identity === 'alice') return hosts.alice
      if (identity === 'bob') return hosts.bob
      throw new Error('identity metadata is required for shell access')
    },
    systemPrompt: () => 'test',
  }
}

test('action metadata switches Host while the same Host keeps its shell state', async () => {
  const hosts = await createHosts()
  const provider = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () =>
      new StubProvider([
        [events.toolCall('alice-1', 'shell_exec', { script: 'mkdir nested && cd nested && pwd', timeoutMs: 1_000 })],
        [events.text('done'), events.response()],
        [events.toolCall('bob-1', 'shell_exec', { script: 'pwd', timeoutMs: 1_000 })],
        [events.text('done'), events.response()],
        [events.toolCall('alice-2', 'shell_exec', { script: 'pwd', timeoutMs: 1_000 })],
        [events.text('done'), events.response()],
      ]),
  })
  const server = new AgentServer({ shellEnvironment: hostlessShellFactory, agent: routedHarness(hosts), providers: [provider] })
  const client = server.client()
  const shellOutputs: ClientSessionEvent[] = []
  client.subscribe((event) => {
    if (event.type === 'shell_output') shellOutputs.push(event)
  })
  await client.open(selection, hosts.base.defaultCwd, globalThis.crypto.randomUUID())

  await client.send([{ type: 'text', text: 'alice first' }], { metadata: { identity: 'alice' } })
  await client.send([{ type: 'text', text: 'bob' }], { metadata: { identity: 'bob' } })
  await client.send([{ type: 'text', text: 'alice again' }], { metadata: { identity: 'alice' } })

  const output = shellOutputs
    .filter((event) => event.type === 'shell_output' && event.status.status === 'exited')
    .map((event) => (event.type === 'shell_output' ? event.status.stdout.delta.trim() : ''))
  expect(output).toEqual([join(hosts.paths.alice, 'nested'), hosts.paths.bob, join(hosts.paths.alice, 'nested')])

  await client.close()
  await server.close()
})

test('a command handle cannot be controlled from another action Host', async () => {
  const hosts = await createHosts()
  const provider = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () =>
      new StubProvider([
        [
          events.toolCall('alice-running', 'shell_exec', {
            script: 'probe stdin',
            timeoutMs: 1,
          }),
        ],
        [events.text('waiting'), events.response()],
      ]),
  })
  const commandId = 'alice-command'
  let commandIndex = 0
  const server = new AgentServer({
    shellEnvironment: hostlessShellFactory,
    agent: routedHarness(hosts),
    providers: [provider],
    shell: { commandIdFactory: () => (commandIndex++ === 0 ? commandId : `${commandId}-${commandIndex}`) },
  })
  const client = server.client()
  await client.open(selection, hosts.base.defaultCwd, globalThis.crypto.randomUUID())
  await client.send([{ type: 'text', text: 'start' }], { metadata: { identity: 'alice' } })

  await expect(
    client.shellWrite(commandId, 'wrong\n', { metadata: { identity: 'bob' } }),
  ).rejects.toThrow('belongs to a different Host')
  await expect(
    client.shellWrite(commandId, 'right\n', { metadata: { identity: 'alice' } }),
  ).resolves.toBeUndefined()

  await client.close()
  await server.close()
})
