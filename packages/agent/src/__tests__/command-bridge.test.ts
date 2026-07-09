import { mkdtemp, readFile, readlink, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import type { Command } from '@demicodes/shell'
import { LocalHost } from '@demicodes/host-local'
import { defineProvider, type Provider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import { AgentServer, CommandBridgeSessionNotFoundError, type AgentHarness } from '../index'
import { materializeCommandBridgeShims } from '../command-bridge-shim'

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

import { z } from 'zod'

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
    name: 'bridge-test',
    initialState: () => ({}),
    host: () => host,
    systemPrompt: () => 'test',
    commands: () => commands(),
  }
}

test('materializeCommandBridgeShims writes an executable dispatch script and one symlink per command name', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-shim-'))
  const host = new LocalHost(cwd)
  const shimSource = '#!/usr/bin/env node\nconsole.log("stub")\n'

  const shimDir = await materializeCommandBridgeShims(host, 'sess-1', ['greet', 'fail'], shimSource)

  const dispatchPath = join(shimDir, '.dispatch')
  const dispatchStat = await stat(dispatchPath)
  expect(dispatchStat.mode & 0o111).not.toBe(0)
  expect(await readFile(dispatchPath, 'utf8')).toBe(shimSource)
  expect(await readFile(join(shimDir, 'package.json'), 'utf8')).toBe('{"type":"commonjs"}\n')

  expect(await readlink(join(shimDir, 'greet'))).toBe('.dispatch')
  expect(await readlink(join(shimDir, 'fail'))).toBe('.dispatch')
})

test('AgentServer.runCommandLine runs a registered command with full stdout/stderr', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-run-'))
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
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-missing-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  const server = new AgentServer({ agent: harness, providers: [stubProvider()] })

  await expect(server.runCommandLine('no-such-session', 'greet', [], { cwd, stdin: '' })).rejects.toBeInstanceOf(
    CommandBridgeSessionNotFoundError,
  )

  await server.close()
})

test('open without commandBridge never materializes shim directories', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-off-'))
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  const server = new AgentServer({ agent: harness, providers: [stubProvider()] })
  const client = server.client()
  await client.open(selection, cwd, globalThis.crypto.randomUUID())

  await expect(stat(join(cwd, '.demi-bin'))).rejects.toThrow()

  await client.close()
  await server.close()
})

test('open with commandBridge prepends shim dir to PATH and sets DEMI_COMMAND_BRIDGE_SOCK', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-bridge-env-'))
  const socketPath = join(cwd, 'bridge.sock')
  const harness = harnessWithCommands(cwd, () => [greetCommand()])
  const server = new AgentServer({
    agent: harness,
    providers: [stubProvider()],
    shell: { initialEnv: { PATH: '/usr/bin' } },
    commandBridge: { socketPath, shimSource: '#!/usr/bin/env node\n' },
  })
  const client = server.client()
  const sessionId = globalThis.crypto.randomUUID()
  await client.open(selection, cwd, sessionId)

  const env = await server.runCommandLine(sessionId, 'greet', ['hello', 'x'], {
    cwd,
    stdin: '',
  })
  expect(env.exitCode).toBe(0)

  // PATH materialization is verified by symlink existence under .demi-bin
  const shimDir = join(cwd, '.demi-bin', sessionId)
  expect(await readlink(join(shimDir, 'greet'))).toBe('.dispatch')

  await client.close()
  await server.close()
})
