import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment, CommandRegistry, type Command } from '../index'
import { LocalHost } from '@demicodes/host-local'

const echoUserSpec: Command = {
  name: 'whoenv',
  summary: 'Echo TEST_USER_ID from the shell env.',
  subcommands: [
    {
      name: 'show',
      summary: 'Print TEST_USER_ID.',
      examples: [],
      run: async (ctx) => {
        await ctx.io.stdout(ctx.env.TEST_USER_ID ?? '(none)')
        return { exitCode: 0 }
      },
    },
  ],
}

function makeEnv(root: string, shellId: string, execEnv: (agentSessionId: string) => Record<string, string>): BashEnvironment {
  const commands = new CommandRegistry()
  commands.register(echoUserSpec)
  return new BashEnvironment({
    host: new LocalHost(root),
    commands,
    shellIdFactory: () => shellId,
    initialEnv: { PATH: process.env.PATH ?? '' },
    execEnv,
  })
}

test('execEnv vars reach registered commands and re-evaluate on every exec', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-exec-env-'))
  let currentUser = 'user-a'
  const env = makeEnv(root, 'shell-exec-env', (agentSessionId) => ({
    TEST_USER_ID: `${currentUser}@${agentSessionId}`,
  }))
  const first = await env.exec({ script: 'whoenv show', agentSessionId: 'conv-1' })
  expect(first.status).toBe('exited')
  if (first.status !== 'exited') throw new Error('expected exited')
  expect(first.stdout.delta).toBe('user-a@conv-1')

  currentUser = 'user-b'
  const second = await env.exec({ script: 'whoenv show', agentSessionId: 'conv-1' })
  expect(second.status).toBe('exited')
  if (second.status !== 'exited') throw new Error('expected exited')
  expect(second.stdout.delta).toBe('user-b@conv-1')
})

test('execEnv vars are exported to external processes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-exec-env-export-'))
  const env = makeEnv(root, 'shell-exec-env-export', () => ({ TEST_USER_ID: 'exported-user' }))
  const result = await env.exec({ script: 'node -p "process.env.TEST_USER_ID"', agentSessionId: 'conv-1' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta.trim()).toBe('exported-user')
})

test('execEnv is not evaluated for anonymous execs', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-exec-env-anon-'))
  let calls = 0
  const env = makeEnv(root, 'shell-exec-env-anon', () => {
    calls += 1
    return { TEST_USER_ID: 'should-not-appear' }
  })
  const result = await env.exec({ script: 'whoenv show' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta).toBe('(none)')
  expect(calls).toBe(0)
})
