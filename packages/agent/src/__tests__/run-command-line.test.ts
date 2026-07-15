import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { z } from 'zod'
import type { ModelSelection } from '@demicodes/core'
import type { Command } from '@demicodes/shell'
import { LocalHost } from '@demicodes/host-local'
import { defineProvider, type Provider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import {
  AgentServer,
  RunCommandLineCommandNotRegisteredError,
  RunCommandLineShellNotFoundError,
  type AgentHarness,
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
const selection = { providerId: 'stub', model }

function stubProvider(): Provider {
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

async function createShell(client: ReturnType<AgentServer['client']>): Promise<void> {
  await client.send([{ type: 'text', text: 'create shell' }])
}

function shellIds(first: string): () => string {
  let index = 0
  return () => (index++ === 0 ? first : `${first}-${index}`)
}

function greetCommand(): Command {
  return {
    name: 'greet',
    summary: 'test command',
    subcommands: [
      {
        name: 'hello',
        summary: 'echoes its argument and stdin',
        positionals: ['name'],
        input: { name: z.string() },
        examples: [],
        run: async (ctx) => {
          const name = String(ctx.parsed.values.name ?? '')
          await ctx.io.stdout(`hello ${name}\n`)
          if (ctx.stdin.text) await ctx.io.stdout(`stdin:${ctx.stdin.text}`)
          return { exitCode: 0 }
        },
      },
    ],
  }
}

function failingCommand(): Command {
  return {
    name: 'fail',
    summary: 'test command that exits non-zero',
    subcommands: [
      {
        name: 'now',
        summary: 'fails',
        examples: [],
        run: async (ctx) => {
          await ctx.io.stderr('boom\n')
          return { exitCode: 7 }
        },
      },
    ],
  }
}

function harnessWithCommands(cwd: string, commands: () => Command[]): AgentHarness<Record<string, never>> {
  const host = new LocalHost(cwd)
  return {
    name: 'run-command-line-test',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'test',
    commands: () => commands(),
  }
}

test('AgentServer.runCommandLine runs a registered command with full stdout/stderr', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-run-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand(), failingCommand()])
  const shellId = 'run-command-line-shell'
  const server = new AgentServer({ agent: harness, providers: [stubProvider()], shell: { shellIdFactory: shellIds(shellId) } })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)
  await createShell(client)

  const ok = await server.runCommandLine(shellId, 'greet', ['hello', 'world'], { cwd, stdin: 'piped' })
  expect(ok.exitCode).toBe(0)
  expect(ok.stdout).toContain('hello world')
  expect(ok.stdout).toContain('stdin:piped')

  const fail = await server.runCommandLine(shellId, 'fail', ['now'], { cwd, stdin: '' })
  expect(fail.exitCode).toBe(7)
  expect(fail.stderr).toContain('boom')

  await client.close()
  await server.close()
})

test('AgentServer.runCommandLine delivers newline-terminated stdin byte-identical', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-stdin-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  const shellId = 'run-command-line-stdin-shell'
  const server = new AgentServer({ agent: harness, providers: [stubProvider()], shell: { shellIdFactory: shellIds(shellId) } })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)
  await createShell(client)

  // What a real pipe delivers: `printf 'line1\nline2\n' | greet hello x`.
  const piped = await server.runCommandLine(shellId, 'greet', ['hello', 'x'], { cwd, stdin: 'line1\nline2\n' })
  expect(piped.stdout).toBe('hello x\nstdin:line1\nline2\n')

  // Stdin without a trailing newline gains exactly one (heredoc normalization).
  const bare = await server.runCommandLine(shellId, 'greet', ['hello', 'x'], { cwd, stdin: 'abc' })
  expect(bare.stdout).toBe('hello x\nstdin:abc\n')

  await client.close()
  await server.close()
})

test('AgentServer.runCommandLine returns binary stdout as base64', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-binary-'))
  const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe])
  const binaryCommand: Command = {
    name: 'blob',
    summary: 'emit binary',
    subcommands: [
      {
        name: 'emit',
        summary: 'write raw bytes to stdout',
        examples: [],
        run: async (ctx) => {
          await ctx.io.stdout(bytes)
          return { exitCode: 0 }
        },
      },
    ],
  }
  const harness = harnessWithCommands(cwd, () => [binaryCommand])
  const shellId = 'run-command-line-binary-shell'
  const server = new AgentServer({ agent: harness, providers: [stubProvider()], shell: { shellIdFactory: shellIds(shellId) } })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)
  await createShell(client)

  const result = await server.runCommandLine(shellId, 'blob', ['emit'], { cwd, stdin: '' })
  expect(result.exitCode).toBe(0)
  expect(result.stdoutEncoding).toBe('base64')
  expect(Uint8Array.from(Buffer.from(result.stdout, 'base64'))).toEqual(bytes)

  await client.close()
  await server.close()
})

test('AgentServer.runCommandLine rejects unknown shells', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-missing-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  const server = new AgentServer({ agent: harness, providers: [stubProvider()] })

  await expect(server.runCommandLine('no-such-shell', 'greet', [], { cwd, stdin: '' })).rejects.toBeInstanceOf(
    RunCommandLineShellNotFoundError,
  )

  await server.close()
})

test('AgentServer.runCommandLine rejects commands not registered for the session', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-unregistered-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  const shellId = 'run-command-line-unregistered-shell'
  const server = new AgentServer({ agent: harness, providers: [stubProvider()], shell: { shellIdFactory: shellIds(shellId) } })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)
  await createShell(client)

  await expect(server.runCommandLine(shellId, 'sh', ['-c', 'echo bypassed'], { cwd, stdin: '' })).rejects.toBeInstanceOf(
    RunCommandLineCommandNotRegisteredError,
  )

  await client.close()
  await server.close()
})

test('prepareShell can inject env without AgentServer knowing about bridges', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-prep-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  let seenNames: readonly string[] = []
  const server = new AgentServer({
    agent: harness,
    providers: [stubProvider()],
    shell: {
      initialEnv: { PATH: '/usr/bin', MARKER: 'base' },
      shellIdFactory: shellIds('run-command-line-prepared-shell'),
    },
    prepareShell: ({ commandNames, shell }) => {
      seenNames = commandNames
      return {
        ...shell,
        initialEnv: {
          ...shell.initialEnv,
          MARKER: 'prepared',
          EXTRA: 'from-hook',
        },
      }
    },
  })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)
  await createShell(client)

  expect(seenNames).toContain('greet')
  const env = await server.runCommandLine('run-command-line-prepared-shell', 'greet', ['hello', 'x'], { cwd, stdin: '' })
  expect(env.exitCode).toBe(0)

  await client.close()
  await server.close()
})
