import { execFile, spawn } from 'node:child_process'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'
import { expect, test } from 'bun:test'
import { z } from 'zod'
import type { ModelSelection } from '@demicodes/core'
import type { Command } from '@demicodes/shell'
import { LocalHost } from '@demicodes/host-local'
import { defineProvider, type Provider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentServer, type AgentHarness } from '../index'
import { COMMAND_BRIDGE_SHIM_SOURCE, startCommandBridge } from '../command-bridge'

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
        summary: 'echoes its argument',
        positionals: ['arg'],
        input: { arg: z.string() },
        examples: [],
        run: async (ctx) => {
          await ctx.io.stdout(`echoed:${ctx.parsed.values.arg ?? ''}\n`)
          if (ctx.stdin.text) await ctx.io.stdout(`stdin:${ctx.stdin.text}`)
          return { exitCode: 0 }
        },
      },
    ],
  }
}

async function socketPathFor(name: string): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'demi-bridge-sock-'))
  return join(dir, `${name}.sock`)
}

test('startCommandBridge answers a raw POST /run like AgentServer.runCommandLine', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-http-'))
  const sessionId = globalThis.crypto.randomUUID()
  const socketPath = await socketPathFor('raw')
  const host = new LocalHost(cwd)
  const harness: AgentHarness<Record<string, never>> = {
    name: 'bridge-transport-test',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'test',
    commands: () => [echoCommand()],
  }
  const server = new AgentServer({
    agent: harness,
    providers: [stubProvider()],
    commandBridge: { socketPath, shimSource: COMMAND_BRIDGE_SHIM_SOURCE },
  })
  const client = server.client()
  await client.open(selection, cwd, sessionId)
  const bridge = startCommandBridge(server, { socketPath })

  try {
    const body = JSON.stringify({
      commandScopeId: sessionId,
      name: 'echo-args',
      args: ['run', 'hi'],
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
    expect(parsed.stdout).toContain('echoed:hi')
  } finally {
    await bridge.close()
    await client.close()
    await server.close()
  }
})

test('a real Node child can bareword-invoke a registered command through the shim PATH', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-child-'))
  const sessionId = globalThis.crypto.randomUUID()
  const socketPath = await socketPathFor('child')
  const host = new LocalHost(cwd)
  const harness: AgentHarness<Record<string, never>> = {
    name: 'bridge-child-test',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'test',
    commands: () => [echoCommand()],
  }
  const server = new AgentServer({
    agent: harness,
    providers: [stubProvider()],
    shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
    commandBridge: { socketPath, shimSource: COMMAND_BRIDGE_SHIM_SOURCE },
  })
  const client = server.client()
  await client.open(selection, cwd, sessionId)
  const bridge = startCommandBridge(server, { socketPath })

  try {
    // Invoke the session shim the same way a nested node process would: PATH + env from the session open.
    const shimDir = join(cwd, '.demi-bin', sessionId)
    const { stdout } = await execFileAsync(join(shimDir, 'echo-args'), ['run', 'from-child'], {
      cwd,
      env: {
        ...process.env,
        PATH: `${shimDir}:${process.env.PATH ?? ''}`,
        DEMI_COMMAND_BRIDGE_SOCK: socketPath,
        DEMI_SESSION_ID: sessionId,
      },
      encoding: 'utf8',
    })
    expect(stdout).toContain('echoed:from-child')
  } finally {
    await bridge.close()
    await client.close()
    await server.close()
  }
})

test('stdin piped to the shim is delivered to the registered command', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-stdin-'))
  const sessionId = globalThis.crypto.randomUUID()
  const socketPath = await socketPathFor('stdin')
  const host = new LocalHost(cwd)
  const harness: AgentHarness<Record<string, never>> = {
    name: 'bridge-stdin-test',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'test',
    commands: () => [echoCommand()],
  }
  const server = new AgentServer({
    agent: harness,
    providers: [stubProvider()],
    shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
    commandBridge: { socketPath, shimSource: COMMAND_BRIDGE_SHIM_SOURCE },
  })
  const client = server.client()
  await client.open(selection, cwd, sessionId)
  const bridge = startCommandBridge(server, { socketPath })

  try {
    const shimDir = join(cwd, '.demi-bin', sessionId)
    const child = spawn(join(shimDir, 'echo-args'), ['run', 'x'], {
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
    expect(stdout).toContain('echoed:x')
    expect(stdout).toContain('stdin:hello-stdin')
  } finally {
    await bridge.close()
    await client.close()
    await server.close()
  }
})
