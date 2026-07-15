/**
 * End-to-end acceptance for composed main:
 * open-box LocalHost bridge + coding harness + web AgentHub path.
 */
import { execFile, spawn } from 'node:child_process'
import { mkdir, mkdtemp, readlink, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { promisify } from 'node:util'
import { expect, test } from 'bun:test'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost, createLocalAgentServer } from '@demicodes/host-local'
import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentWorkspace } from '@demicodes/web-ui/agent/workspace'
import { connectControlClient } from '@demicodes/web-ui/transport/control-client'
import { parseServerOptions } from '../server-options'
import { startWebServer } from '../serve'
import { createStubProvider } from '../stub-provider'

const execFileAsync = promisify(execFile)

const model = {
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

async function shortDirs(tag: string) {
  const cwd = await mkdtemp(join('/tmp', `e2e-${tag}-`))
  const stateDir = join('/tmp', `e2e-st-${tag}-${Date.now().toString(36)}`)
  await mkdir(stateDir, { recursive: true })
  return { cwd, stateDir, socketPath: join(stateDir, 'bridges', 'e2e.sock') }
}

test('e2e accept: LocalHost bridge materializes under stateDir; OS child + runCommandLine run coding commands', async () => {
  const { cwd, stateDir, socketPath } = await shortDirs('bridge')
  const host = new LocalHost(cwd)
  const harness = createCodingAgentHarness({ host })
  const shellId = 'e2e-bridge-shell'
  let shellIndex = 0
  const stub = defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () =>
      new StubProvider([
        [events.toolCall('create-shell', 'shell_exec', { script: 'printf ready', timeoutMs: 1_000 })],
        [events.text('ready'), events.response()],
        [events.text('e2e-ok'), events.response()],
      ]),
  })
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness,
    providers: [stub],
    stateDir,
    commandBridgeSocketPath: socketPath,
    shell: { shellIdFactory: () => (shellIndex++ === 0 ? shellId : `${shellId}-${shellIndex}`) },
  })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  try {
    await client.open(selection, cwd, sessionId)
    await client.send([{ type: 'text', text: 'create shell' }])

    await expect(stat(join(cwd, '.demi-bin'))).rejects.toThrow()
    await expect(stat(join(cwd, '.demi'))).rejects.toThrow()
    expect(await readlink(join(stateDir, 'bridge-bin', sessionId, 'todo'))).toBe('.dispatch')
    expect(await readlink(join(stateDir, 'bridge-bin', sessionId, 'demi'))).toBe('.dispatch')

    const list = await server.runCommandLine(shellId, 'todo', ['list'], { cwd, stdin: '' })
    expect(list.exitCode).toBe(0)

    const shimDir = join(stateDir, 'bridge-bin', sessionId)
    const { stdout } = await execFileAsync(join(shimDir, 'todo'), ['list'], {
      cwd,
      env: {
        ...process.env,
        PATH: `${shimDir}:${process.env.PATH ?? ''}`,
        DEMI_COMMAND_BRIDGE_SOCK: socketPath,
        DEMI_SHELL_ID: shellId,
      },
      encoding: 'utf8',
    })
    expect(typeof stdout).toBe('string')

    const child = spawn(join(shimDir, 'todo'), ['list'], {
      cwd,
      env: {
        ...process.env,
        PATH: `${shimDir}:${process.env.PATH ?? ''}`,
        DEMI_COMMAND_BRIDGE_SOCK: socketPath,
        DEMI_SHELL_ID: shellId,
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    const code = await new Promise<number | null>((resolve, reject) => {
      child.on('error', reject)
      child.on('close', resolve)
    })
    expect(code).toBe(0)

    await client.send([{ type: 'text', text: 'ping' }])
  } finally {
    await client.close().catch(() => {})
    await close()
  }
})

test('e2e accept: web AgentHub (createLocalAgentServer) websocket conversation with stub provider', async () => {
  const cwd = await mkdtemp(join('/tmp', 'e2e-web-'))
  const handle = startWebServer([createStubProvider()], { ...parseServerOptions(['--cwd', cwd]), port: 0 })
  try {
    const control = await connectControlClient(`${handle.url.replace(/^http/, 'ws')}/control`)
    const workspace = new AgentWorkspace({ baseUrl: handle.url, control, cwd })
    await workspace.init()
    const id = workspace.activeId.value
    expect(id).not.toBeNull()

    await workspace.send(id!, [{ type: 'text', text: 'e2e hello' }])
    const state = workspace.sessions[id!]!
    expect(state.phase).toBe('idle')
    expect(
      state.blocks.some((block) => block.type === 'text' && block.text.includes('Hello from the stub provider.')),
    ).toBe(true)

    await workspace.dispose()
  } finally {
    await handle.stop()
  }
})

test('e2e accept: providers factory includes grok-build without throwing', async () => {
  const { createWebProviders } = await import('../providers')
  const options = parseServerOptions(['--cwd', process.cwd(), '--provider', 'grok-build'])
  const providers = createWebProviders(options)
  const ids = providers.map((p) => p.id)
  expect(ids).toContain('grok-build')
  expect(ids).toContain('codex')
  expect(ids[0]).toBe('grok-build')
})
