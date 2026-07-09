import { mkdtemp, readlink, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import type { AgentHarness } from '@demicodes/agent'
import type { Command } from '@demicodes/shell'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { LocalHost, createLocalAgentServer } from '../index'

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

function demoCommand(): Command {
  return {
    name: 'pingcmd',
    summary: 'ping',
    subcommands: [
      {
        name: 'run',
        summary: 'pong',
        examples: [],
        run: async ({ io }) => {
          await io.stdout('pong\n')
          return { exitCode: 0 }
        },
      },
    ],
  }
}

function harness(host: LocalHost): AgentHarness<Record<string, never>> {
  return {
    name: 'local-box',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'test',
    commands: () => [demoCommand()],
  }
}

function stubProvider() {
  return defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () => new StubProvider([[events.text('hi'), events.response()]]),
  })
}

test('createLocalAgentServer enables command bridge by default', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-local-box-on-'))
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host),
    providers: [stubProvider()],
  })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)

  const shim = join(cwd, '.demi-bin', sessionId, 'pingcmd')
  expect(await readlink(shim)).toBe('.dispatch')

  await client.close()
  await close()
})

test('createLocalAgentServer commandBridge: false leaves no shim directory', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-local-box-off-'))
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host),
    providers: [stubProvider()],
    commandBridge: false,
  })
  const client = server.client()
  await client.open(selection, cwd, globalThis.crypto.randomUUID())

  await expect(stat(join(cwd, '.demi-bin'))).rejects.toThrow()

  await client.close()
  await close()
})

test('createLocalAgentServer default bridge: bareword shim works via runCommandLine path', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-local-box-run-'))
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host),
    providers: [stubProvider()],
  })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)

  const result = await server.runCommandLine(sessionId, 'pingcmd', ['run'], { cwd, stdin: '' })
  expect(result.exitCode).toBe(0)
  expect(result.stdout).toContain('pong')

  await client.close()
  await close()
})
