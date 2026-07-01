import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment, CommandRegistry, type CommandSpec } from '../index'
import { LocalHost } from '@demicodes/host-local'

const echoSessionSpec: CommandSpec = {
  name: 'sessionid',
  summary: 'Echo the agent session id from the shell env.',
  subcommands: [
    {
      name: 'show',
      summary: 'Print DEMI_AGENT_SESSION_ID.',
      examples: [],
      run: async (ctx) => {
        await ctx.io.stdout(ctx.env.DEMI_AGENT_SESSION_ID ?? '(none)')
        return { exitCode: 0 }
      },
    },
  ],
}

function makeEnv(root: string, shellId: string): BashEnvironment {
  const commands = new CommandRegistry()
  commands.register(echoSessionSpec)
  return new BashEnvironment({
    host: new LocalHost(root),
    commands,
    shellIdFactory: () => shellId,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
}

test('registered commands see DEMI_AGENT_SESSION_ID from exec agentSessionId', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-session-env-'))
  const env = makeEnv(root, 'shell-session-env')
  const result = await env.exec({ script: 'sessionid show', agentSessionId: 'conv-abc-123' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta).toBe('conv-abc-123')
})

test('anonymous exec (no agentSessionId) leaves DEMI_AGENT_SESSION_ID unset', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-session-env-none-'))
  const env = makeEnv(root, 'shell-session-env-none')
  const result = await env.exec({ script: 'sessionid show' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta).toBe('(none)')
})
