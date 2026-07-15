import { mkdtemp, mkdir, readlink, stat } from 'node:fs/promises'
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
    createRuntime: () =>
      new StubProvider([
        [events.toolCall('create-shell', 'shell_exec', { script: 'printf ready', timeoutMs: 1_000 })],
        [events.text('ready'), events.response()],
      ]),
  })
}

/** Short paths under /tmp — macOS AF_UNIX path limit is ~104 bytes. */
async function shortDirs(tag: string) {
  const cwd = await mkdtemp(join('/tmp', `dc-${tag}-`))
  const stateDir = join('/tmp', `ds-${tag}-${Date.now().toString(36)}`)
  await mkdir(stateDir, { recursive: true })
  return { cwd, stateDir }
}

test('createLocalAgentServer enables command bridge by default under stateDir, not cwd', async () => {
  const { cwd, stateDir } = await shortDirs('on')
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host),
    providers: [stubProvider()],
    stateDir,
  })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)
  await client.send([{ type: 'text', text: 'create shell' }])

  await expect(stat(join(cwd, '.demi-bin'))).rejects.toThrow()
  await expect(stat(join(cwd, '.demi'))).rejects.toThrow()
  expect(await readlink(join(stateDir, 'bridge-bin', sessionId, 'pingcmd'))).toBe('.dispatch')

  await client.close()
  await close()
})

test('createLocalAgentServer commandBridge: false leaves no shim directory', async () => {
  const { cwd, stateDir } = await shortDirs('off')
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host),
    providers: [stubProvider()],
    commandBridge: false,
    stateDir,
  })
  const client = server.client()
  await client.open(selection, cwd, globalThis.crypto.randomUUID())

  await expect(stat(join(cwd, '.demi-bin'))).rejects.toThrow()
  await expect(stat(join(stateDir, 'bridge-bin'))).rejects.toThrow()

  await client.close()
  await close()
})

test('createLocalAgentServer default bridge: runCommandLine works', async () => {
  const { cwd, stateDir } = await shortDirs('run')
  const host = new LocalHost(cwd)
  const shellId = 'local-server-shell'
  let shellIndex = 0
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host),
    providers: [stubProvider()],
    stateDir,
    shell: { shellIdFactory: () => (shellIndex++ === 0 ? shellId : `${shellId}-${shellIndex}`) },
  })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)
  await client.send([{ type: 'text', text: 'create shell' }])

  const result = await server.runCommandLine(shellId, 'pingcmd', ['run'], { cwd, stdin: '' })
  expect(result.exitCode).toBe(0)
  expect(result.stdout).toContain('pong')

  await client.close()
  await close()
})
