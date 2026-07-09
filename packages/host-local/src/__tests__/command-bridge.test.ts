import { execFile, spawn } from 'node:child_process'
import { mkdtemp, mkdir, readFile, readlink, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { promisify } from 'node:util'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import type { AgentHarness } from '@demicodes/agent'
import type { Command } from '@demicodes/shell'
import { defineProvider, type Provider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import {
  COMMAND_BRIDGE_SHIM_SOURCE,
  LocalHost,
  createLocalAgentServer,
  materializeCommandBridgeShims,
  startCommandBridge,
} from '../index'

const execFileAsync = promisify(execFile)

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

function stubProvider(): Provider {
  return defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () => new StubProvider([[events.text('hi'), events.response()]]),
  })
}

function echoCommand(): Command {
  return {
    name: 'echo-args',
    summary: 'echoes CWD-independent test output',
    subcommands: [
      {
        name: 'run',
        summary: 'echoes a fixed token and optional stdin',
        examples: [],
        run: async (ctx) => {
          await ctx.io.stdout('echoed:ok\n')
          if (ctx.stdin.text) await ctx.io.stdout(`stdin:${ctx.stdin.text}`)
          return { exitCode: 0 }
        },
      },
    ],
  }
}

function harness(host: LocalHost, commands: () => Command[]): AgentHarness<Record<string, never>> {
  return {
    name: 'bridge-test',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'test',
    commands: () => commands(),
  }
}

/** Short paths under /tmp — macOS AF_UNIX path limit is ~104 bytes. */
async function shortDirs(tag: string) {
  const cwd = await mkdtemp(join('/tmp', `dc-${tag}-`))
  const stateDir = join('/tmp', `ds-${tag}-${Date.now().toString(36)}`)
  await mkdir(stateDir, { recursive: true })
  return {
    cwd,
    stateDir,
    socketPath: join(stateDir, 'bridges', 'b.sock'),
  }
}

test('materializeCommandBridgeShims writes under stateDir/bridge-bin, not workspace cwd', async () => {
  const { cwd, stateDir } = await shortDirs('shim')
  const host = new LocalHost(cwd)
  const shimSource = '#!/usr/bin/env node\nconsole.log("stub")\n'

  const shimDir = await materializeCommandBridgeShims({
    host,
    agentSessionId: 'sess-1',
    commandNames: ['greet', 'fail'],
    shimSource,
    stateDir,
  })

  expect(shimDir.includes(join(stateDir, 'bridge-bin'))).toBe(true)
  await expect(stat(join(cwd, '.demi-bin'))).rejects.toThrow()
  await expect(stat(join(cwd, '.demi'))).rejects.toThrow()

  expect(await readFile(join(shimDir, '.dispatch'), 'utf8')).toBe(shimSource)
  expect(await readlink(join(shimDir, 'greet'))).toBe('.dispatch')
  expect(await readlink(join(shimDir, 'fail'))).toBe('.dispatch')
})

test('startCommandBridge answers a raw POST /run like AgentServer.runCommandLine', async () => {
  const { cwd, stateDir, socketPath } = await shortDirs('http')
  const sessionId = globalThis.crypto.randomUUID()
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host, () => [echoCommand()]),
    providers: [stubProvider()],
    stateDir,
    commandBridgeSocketPath: socketPath,
  })
  const client = server.client()
  await client.open(selection, cwd, sessionId)

  try {
    const body = JSON.stringify({
      commandScopeId: sessionId,
      name: 'echo-args',
      args: ['run'],
      cwd,
      stdin: '',
    })
    const { request } = await import('node:http')
    const response = await new Promise<{ status: number; body: string }>((resolve, reject) => {
      const req = request(
        { socketPath, path: '/run', method: 'POST', headers: { 'content-type': 'application/json' } },
        (res) => {
          const chunks: Buffer[] = []
          res.on('data', (c) => chunks.push(c))
          res.on('end', () => resolve({ status: res.statusCode ?? 0, body: Buffer.concat(chunks).toString('utf8') }))
        },
      )
      req.on('error', reject)
      req.end(body)
    })
    expect(response.status).toBe(200)
    const parsed = JSON.parse(response.body) as { exitCode: number; stdout: string }
    expect(parsed.exitCode).toBe(0)
    expect(parsed.stdout).toContain('echoed:ok')
  } finally {
    await client.close()
    await close()
  }
})

test('a real Node child can bareword-invoke a registered command through the shim PATH', async () => {
  const { cwd, stateDir, socketPath } = await shortDirs('child')
  const sessionId = globalThis.crypto.randomUUID()
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host, () => [echoCommand()]),
    providers: [stubProvider()],
    stateDir,
    commandBridgeSocketPath: socketPath,
  })
  const client = server.client()
  await client.open(selection, cwd, sessionId)

  try {
    const shimDir = join(stateDir, 'bridge-bin', sessionId)
    const { stdout } = await execFileAsync(join(shimDir, 'echo-args'), ['run'], {
      cwd,
      env: {
        ...process.env,
        PATH: `${shimDir}:${process.env.PATH ?? ''}`,
        DEMI_COMMAND_BRIDGE_SOCK: socketPath,
        DEMI_SESSION_ID: sessionId,
      },
      encoding: 'utf8',
    })
    expect(stdout).toContain('echoed:ok')
  } finally {
    await client.close()
    await close()
  }
})

test('stdin piped to the shim is delivered to the registered command', async () => {
  const { cwd, stateDir, socketPath } = await shortDirs('stdin')
  const sessionId = globalThis.crypto.randomUUID()
  const host = new LocalHost(cwd)
  const { server, close } = createLocalAgentServer({
    host,
    agent: harness(host, () => [echoCommand()]),
    providers: [stubProvider()],
    stateDir,
    commandBridgeSocketPath: socketPath,
  })
  const client = server.client()
  await client.open(selection, cwd, sessionId)

  try {
    const shimDir = join(stateDir, 'bridge-bin', sessionId)
    const child = spawn(join(shimDir, 'echo-args'), ['run'], {
      cwd,
      env: {
        ...process.env,
        PATH: `${shimDir}:${process.env.PATH ?? ''}`,
        DEMI_COMMAND_BRIDGE_SOCK: socketPath,
        DEMI_SESSION_ID: sessionId,
      },
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    child.stdin!.write('hello-stdin')
    child.stdin!.end()
    const stdout = await new Promise<string>((resolve, reject) => {
      const chunks: Buffer[] = []
      child.stdout!.on('data', (c) => chunks.push(c))
      child.on('error', reject)
      child.on('close', (code) => {
        if (code === 0) resolve(Buffer.concat(chunks).toString('utf8'))
        else reject(new Error(`exit ${code}`))
      })
    })
    expect(stdout).toContain('echoed:ok')
    expect(stdout).toContain('stdin:hello-stdin')
  } finally {
    await client.close()
    await close()
  }
})

test('startCommandBridge alone can be wired without createLocalAgentServer materialize', async () => {
  // Direct unit path: materialize + prepareSessionShell + startCommandBridge
  // without the factory (still all host-local).
  const { cwd, stateDir, socketPath } = await shortDirs('manual')
  const sessionId = globalThis.crypto.randomUUID()
  const host = new LocalHost(cwd)
  const { AgentServer } = await import('@demicodes/agent')
  const server = new AgentServer({
    agent: harness(host, () => [echoCommand()]),
    providers: [stubProvider()],
    shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
    prepareSessionShell: async ({ host: sessionHost, agentSessionId, commandNames, shell }) => {
      const shimDir = await materializeCommandBridgeShims({
        host: sessionHost,
        agentSessionId,
        commandNames,
        shimSource: COMMAND_BRIDGE_SHIM_SOURCE,
        stateDir,
      })
      return {
        ...shell,
        initialEnv: {
          ...shell.initialEnv,
          DEMI_COMMAND_BRIDGE_SOCK: socketPath,
          PATH: `${shimDir}:${shell.initialEnv?.PATH ?? ''}`,
        },
      }
    },
  })
  const client = server.client()
  await client.open(selection, cwd, sessionId)
  const bridge = startCommandBridge(server, { socketPath })
  try {
    expect(await readlink(join(stateDir, 'bridge-bin', sessionId, 'echo-args'))).toBe('.dispatch')
    const result = await server.runCommandLine(sessionId, 'echo-args', ['run'], { cwd, stdin: '' })
    expect(result.stdout).toContain('echoed:ok')
  } finally {
    await bridge.close()
    await client.close()
    await server.close()
  }
})
