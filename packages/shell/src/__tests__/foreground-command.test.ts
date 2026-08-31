import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { decodeUtf8, waitFor } from '@demicodes/utils'
import { LocalHost } from '@demicodes/host-local'
import { BashEnvironment, CommandRegistry, type Command } from '../index'

async function pollUntilSettled(env: BashEnvironment, commandId: string): Promise<Awaited<ReturnType<BashEnvironment['status']>>> {
  let latest = await env.status({ commandId })
  for (let attempt = 0; attempt < 200 && latest.status === 'running'; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, 5))
    latest = await env.status({ commandId })
  }
  return latest
}

async function makeEnv(commands: Command[]): Promise<BashEnvironment> {
  const root = await mkdtemp(join(tmpdir(), 'demi-foreground-command-'))
  const registry = new CommandRegistry()
  for (const command of commands) registry.register(command)
  return new BashEnvironment({
    host: new LocalHost(root),
    commands: registry,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
}

test('a long-running registered command streams live stdout and honors the abort signal', async () => {
  let sawAbort = false
  const env = await makeEnv([
    {
      name: 'pulse',
      summary: 'Writes a line, then waits for the abort signal.',
      run: async ({ io, signal }) => {
        await io.stdout('started\n')
        await new Promise<void>((resolve) => {
          if (signal.aborted) return resolve()
          signal.addEventListener('abort', () => resolve(), { once: true })
        })
        sawAbort = signal.aborted
        return { exitCode: 130 }
      },
    },
  ])

  const started = await env.exec({ script: 'pulse', timeoutMs: 200, agentSessionId: 'fg-live' })
  expect(started.status).toBe('running')
  // Live view: the streamed stdout is visible while the command is still running.
  const observed = await env.status({ commandId: started.commandId })
  expect(observed.status).toBe('running')
  expect(observed.stdout.tail).toContain('started')

  const aborted = await env.abort({ commandId: started.commandId })
  expect(aborted.status).toBe('aborted')
  await waitFor(() => sawAbort)
  expect(sawAbort).toBe(true)
})

test('shell_write chunks arrive on the registered command stdin stream, one chunk per write', async () => {
  const received: string[] = []
  const env = await makeEnv([
    {
      name: 'listen',
      summary: 'Echoes each post-start stdin chunk until "quit".',
      run: async ({ io, stdinStream }) => {
        for await (const chunk of stdinStream) {
          const text = decodeUtf8(chunk)
          received.push(text)
          await io.stdout(`got ${text.trim()}\n`)
          if (text.trim() === 'quit') break
        }
        return { exitCode: 0 }
      },
    },
  ])

  const started = await env.exec({ script: 'listen', timeoutMs: 100, agentSessionId: 'fg-stdin' })
  expect(started.status).toBe('running')

  await env.write({ commandId: started.commandId, stdin: 'first steer\n' })
  await waitFor(() => received.length === 1)
  await env.write({ commandId: started.commandId, stdin: 'quit\n' })
  await waitFor(() => received.length === 2)

  expect(received).toEqual(['first steer\n', 'quit\n'])
  const finished = await pollUntilSettled(env, started.commandId)
  expect(finished.status).toBe('exited')
  if (finished.status !== 'exited') throw new Error('expected exited')
  expect(finished.exitCode).toBe(0)
  expect(finished.stdout.tail).toContain('got first steer')
  expect(finished.stdout.tail).toContain('got quit')
})

test('piped stdin stays the snapshot; registered command pipes remain byte-clean', async () => {
  const env = await makeEnv([
    {
      name: 'shout',
      summary: 'Uppercases piped stdin.',
      run: async ({ stdin, io }) => {
        await io.stdout(stdin.text.toUpperCase())
        return { exitCode: 0 }
      },
    },
  ])

  const result = await env.exec({ script: 'printf "hi there" | shout | tr A-Z a-z', agentSessionId: 'fg-pipe' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe('hi there')
})

test('a running registered command surfaces the executed node runningHint; siblings and exit do not', async () => {
  const env = await makeEnv([
    {
      name: 'attend',
      summary: 'Routes to wait/quick.',
      subcommands: [
        {
          name: 'wait',
          summary: 'Runs until aborted.',
          runningHint: 'next: still attending; do not poll.',
          run: async ({ signal }) => {
            await new Promise<void>((resolve) => {
              if (signal.aborted) return resolve()
              signal.addEventListener('abort', () => resolve(), { once: true })
            })
            return { exitCode: 0 }
          },
        },
        {
          name: 'quick',
          summary: 'Exits immediately.',
          run: async () => ({ exitCode: 0 }),
        },
      ],
    },
  ])

  const started = await env.exec({ script: 'attend wait', timeoutMs: 100, agentSessionId: 'fg-hint' })
  expect(started.status).toBe('running')
  if (started.status !== 'running') throw new Error('expected running')
  expect(started.runningHint).toBe('next: still attending; do not poll.')

  const observed = await env.status({ commandId: started.commandId })
  expect(observed.status === 'running' && observed.runningHint).toBe('next: still attending; do not poll.')

  const aborted = await env.abort({ commandId: started.commandId })
  expect(aborted.status).toBe('aborted')
  expect('runningHint' in aborted).toBe(false)

  const quick = await env.exec({ script: 'attend quick', timeoutMs: 100, agentSessionId: 'fg-hint' })
  expect(quick.status).toBe('exited')
  expect('runningHint' in quick).toBe(false)
})
