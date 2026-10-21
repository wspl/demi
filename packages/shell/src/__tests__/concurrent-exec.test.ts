import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { LocalHost } from '@demicodes/host-local'
import { BashEnvironment, CommandRegistry } from '../index'

for (const isWarm of [false, true]) {
  test(`concurrent execs of one agent session run on separate shells (${isWarm ? 'idle default shell' : 'no default shell yet'})`, async () => {
    const gate = deferred<void>()
    const registry = new CommandRegistry()
    registry.register({
      name: 'gate',
      summary: 'Waits until the test opens the gate.',
      subcommands: [
        {
          name: 'wait',
          summary: 'Wait for the gate.',
          run: async ({ io }) => {
            await gate.promise
            await io.stdout('slow\n')
            return { exitCode: 0 }
          },
        },
      ],
    })
    const root = await mkdtemp(join(tmpdir(), 'demi-bash-concurrent-'))
    const env = new BashEnvironment({ host: new LocalHost(root), commands: registry })
    const agentSessionId = 'session-concurrent'
    if (isWarm) await env.exec({ agentSessionId, script: 'true' })

    const slow = env.exec({ agentSessionId, script: 'gate wait', timeoutMs: 5_000 })
    const fast = await env.exec({ agentSessionId, script: 'echo fast', timeoutMs: 5_000 })
    gate.resolve()
    const slowResult = await slow

    expect(fast).toMatchObject({ status: 'exited', exitCode: 0 })
    expect(slowResult).toMatchObject({ status: 'exited', exitCode: 0 })
    expect(fast.stdout.delta).toBe('fast\n')
    expect(slowResult.stdout.delta).toBe('slow\n')
    expect(slowResult.shellId).not.toBe(fast.shellId)
  })
}
