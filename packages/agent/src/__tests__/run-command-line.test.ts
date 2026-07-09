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
import { AgentServer, RunCommandLineSessionNotFoundError, type AgentHarness } from '../index'

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
  const server = new AgentServer({ agent: harness, providers: [stubProvider()] })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)

  const ok = await server.runCommandLine(sessionId, 'greet', ['hello', 'world'], { cwd, stdin: 'piped' })
  expect(ok.exitCode).toBe(0)
  expect(ok.stdout).toContain('hello world')
  expect(ok.stdout).toContain('stdin:piped')

  const fail = await server.runCommandLine(sessionId, 'fail', ['now'], { cwd, stdin: '' })
  expect(fail.exitCode).toBe(7)
  expect(fail.stderr).toContain('boom')

  await client.close()
  await server.close()
})

test('AgentServer.runCommandLine rejects unknown sessions', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-missing-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  const server = new AgentServer({ agent: harness, providers: [stubProvider()] })

  await expect(server.runCommandLine('no-such-session', 'greet', [], { cwd, stdin: '' })).rejects.toBeInstanceOf(
    RunCommandLineSessionNotFoundError,
  )

  await server.close()
})

test('prepareSessionShell can inject env without AgentServer knowing about bridges', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-rcl-prep-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  let seenNames: readonly string[] = []
  const server = new AgentServer({
    agent: harness,
    providers: [stubProvider()],
    shell: { initialEnv: { PATH: '/usr/bin', MARKER: 'base' } },
    prepareSessionShell: ({ commandNames, shell }) => {
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

  expect(seenNames).toContain('greet')
  const env = await server.runCommandLine(sessionId, 'greet', ['hello', 'x'], { cwd, stdin: '' })
  expect(env.exitCode).toBe(0)

  await client.close()
  await server.close()
})
