import { expect, test } from 'bun:test'
import process from 'node:process'
import { decodeUtf8, encodeUtf8 } from '@demicodes/utils'
import type { InferenceRequest } from '@demicodes/provider'
import type { HostProcess } from '@demicodes/shell'
import {
  ClaudeCliTransportFactory,
  type ClaudeSpawn,
  type ClaudeSpawnHandle,
  type ClaudeSpawnParams,
} from '../transport'

// Compile-time contract: a real `Host.process.spawn` is assignable to the
// structurally-typed injectable spawn (the package must not import shell at
// runtime, so the shapes are duplicated and this assignment guards the drift).
const _hostSpawnIsAssignable: ClaudeSpawn = null as unknown as HostProcess['spawn']
void _hostSpawnIsAssignable

function makeRequest(overrides: Partial<InferenceRequest> = {}): InferenceRequest {
  return {
    sessionId: 'transport-test-session',
    turnId: 'test-turn',
    requestId: 'test-request',
    modelId: 'claude-test',
    systemPrompt: 'system',
    cwd: '/workspace/project',
    items: [],
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
    ...overrides,
  }
}

interface FakeSpawn {
  spawn: ClaudeSpawn
  calls: ClaudeSpawnParams[]
  stdin: Uint8Array[]
  kills: Array<string | undefined>
  closeExit(exit?: { exitCode: number | null; signal?: string }): void
}

function makeFakeSpawn(stdoutParts: string[], stderrParts: string[] = []): FakeSpawn {
  const calls: ClaudeSpawnParams[] = []
  const stdin: Uint8Array[] = []
  const kills: Array<string | undefined> = []
  let resolveExit: (exit: { exitCode: number | null; signal?: string }) => void = () => {}
  const exit = new Promise<{ exitCode: number | null; signal?: string }>((resolve) => {
    resolveExit = resolve
  })

  const handle: ClaudeSpawnHandle = {
    stdout: (async function* () {
      for (const part of stdoutParts) yield encodeUtf8(part)
    })(),
    stderr: (async function* () {
      for (const part of stderrParts) yield encodeUtf8(part)
    })(),
    writeStdin: async (data) => {
      stdin.push(data)
    },
    closeStdin: async () => {},
    kill: async (signal) => {
      kills.push(signal)
      resolveExit({ exitCode: null, signal: signal ?? undefined })
    },
    wait: () => exit,
  }

  return {
    spawn: async (params) => {
      calls.push(params)
      return handle
    },
    calls,
    stdin,
    kills,
    closeExit: (value = { exitCode: 0 }) => resolveExit(value),
  }
}

test('injected spawn receives command, args, untranslated cwd, and a clean env', async () => {
  const fake = makeFakeSpawn([])
  const factory = new ClaudeCliTransportFactory({
    claudePath: '/opt/claude',
    spawn: fake.spawn,
    env: { ANTHROPIC_BASE_URL: 'http://backend/passthrough', CLAUDE_CODE_OAUTH_TOKEN: 'runner-token' },
    resolveOAuthAccessToken: async () => 'vault-oauth-token',
  })

  // cwd does not exist locally; an injected spawn must still receive it as-is.
  await factory.start(makeRequest({ cwd: '/definitely/not/a/local/dir' }))

  expect(fake.calls).toHaveLength(1)
  const params = fake.calls[0]
  expect(params.command).toBe('/opt/claude')
  expect(params.args).toContain('--model')
  expect(params.cwd).toBe('/definitely/not/a/local/dir')

  const env = params.env ?? {}
  // Never leaks the local process env across the wire.
  expect(env.PATH).toBeUndefined()
  expect(Object.keys(env).some((key) => key === 'HOME')).toBe(false)
  // CLI builtins are present; the public overlay wins over the resolved OAuth token.
  expect(env.DISABLE_AUTO_COMPACT).toBe('1')
  expect(env.MAX_MCP_OUTPUT_TOKENS).toBe('1000000')
  expect(env.ANTHROPIC_BASE_URL).toBe('http://backend/passthrough')
  expect(env.CLAUDE_CODE_OAUTH_TOKEN).toBe('runner-token')
})

test('transport round-trips stream-json over an injected spawn handle', async () => {
  const fake = makeFakeSpawn(
    ['{"type":"system"}\n{"type":"assist', 'ant","n":1}\n', '\n{"type":"result"}\n'],
    ['warn', 'ing'],
  )
  const factory = new ClaudeCliTransportFactory({ spawn: fake.spawn })
  const transport = await factory.start(makeRequest())

  await transport.writeJson({ type: 'user', n: 2 })
  expect(fake.stdin).toHaveLength(1)
  expect(decodeUtf8(fake.stdin[0])).toBe('{"type":"user","n":2}\n')

  const messages: unknown[] = []
  for await (const message of transport.messages()) messages.push(message)
  expect(messages).toEqual([{ type: 'system' }, { type: 'assistant', n: 1 }, { type: 'result' }])

  await transport.kill()
  expect(fake.kills).toEqual(['SIGTERM'])
  const exit = await transport.wait()
  expect(exit.exitCode).toBeNull()
  // stderr is collected in the background; the exit above has drained it.
  expect(transport.stderrText()).toBe('warning')
})

test('local default spawn runs and reaps a real child process', async () => {
  const factory = new ClaudeCliTransportFactory({ claudePath: process.execPath })

  // The claude arg vector is not a valid node invocation (node rejects
  // `--output-format`), so the child exits immediately with a nonzero code —
  // which is all this needs: the local wrapper really spawned and reaped it.
  const transport = await factory.start(makeRequest())
  const exit = await transport.wait()
  expect(typeof exit.exitCode).toBe('number')
  expect(exit.exitCode).not.toBe(0)
})
