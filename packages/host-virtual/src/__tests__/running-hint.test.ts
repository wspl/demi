import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { CommandRegistry, runtimeModule, type Command } from '@demicodes/shell'
import { abortable, deferred, waitFor } from '@demicodes/utils'
import { hostlessShell, LocalHost } from '../testing'

async function shellFor(commands: Command[]) {
  const dir = await mkdtemp(join(tmpdir(), 'hostless-running-hint-'))
  const registry = new CommandRegistry()
  for (const command of commands) registry.register(command)
  const shell = await hostlessShell({ host: new LocalHost(dir, { storeRoot: join(dir, '.store') }), commands: registry })
  return {
    shell,
    dispose: async () => {
      await shell.disposeAllShells()
      await rm(dir, { recursive: true, force: true })
    },
  }
}

test('only the executed registered leaf supplies a hint; exit, abort, help and siblings clear it', async () => {
  const held = deferred<void>()
  const plainStarted = deferred<void>()
  const w = await shellFor([{
    name: 'attend', summary: 'Test hint lifetimes.', subcommands: [
      { name: 'wait', summary: 'Wait.', kind: 'rpc', runningHint: 'next: attending; do not poll.', run: async ({ signal }) => { await abortable(held.promise, signal); return { exitCode: 0 } } },
      { name: 'plain', summary: 'Wait without a hint.', kind: 'rpc', run: async ({ signal }) => { plainStarted.resolve(); await abortable(new Promise<void>(() => {}), signal); return { exitCode: 0 } } },
      { name: 'quick', summary: 'Exit.', kind: 'rpc', run: () => ({ exitCode: 0 }) },
      { name: 'fail', summary: 'Throw.', kind: 'rpc', runningHint: 'must clear on failure', run: () => { throw new Error('failed') } },
    ],
  }])
  try {
    const started = await w.shell.exec({ script: 'attend wait; attend plain', timeoutMs: 20 })
    expect(started.status).toBe('running')
    expect(started.status === 'running' && started.runningHint).toBe('next: attending; do not poll.')
    const observed = await w.shell.status({ commandId: started.commandId })
    expect(observed.status === 'running' && observed.runningHint).toBe('next: attending; do not poll.')
    held.resolve()
    await plainStarted.promise
    expect('runningHint' in await w.shell.status({ commandId: started.commandId })).toBe(false)
    const aborted = await w.shell.abort({ commandId: started.commandId })
    expect(aborted.status).toBe('aborted')
    expect('runningHint' in aborted).toBe(false)
    for (const script of ['attend quick', 'attend wait', 'attend wait --help', 'attend wait --unknown', 'attend fail']) {
      const done = await w.shell.exec({ script, timeoutMs: 1_000 })
      expect(done.status).toBe('exited')
      expect('runningHint' in done).toBe(false)
    }
  } finally { await w.dispose() }
})

test('concurrent pipeline invocations keep their own hints until each settles', async () => {
  const first = deferred<void>()
  const second = deferred<void>()
  let ended = false
  const w = await shellFor([{
    name: 'attend', summary: 'Concurrent hints.', subcommands: [
      { name: 'first', summary: 'First.', kind: 'rpc', runningHint: 'first hint', run: async ({ signal }) => { await abortable(first.promise, signal); return { exitCode: 0 } } },
      { name: 'second', summary: 'Second.', kind: 'rpc', runningHint: 'second hint', run: async ({ signal }) => { await abortable(second.promise, signal); ended = true; return { exitCode: 0 } } },
    ],
  }])
  try {
    const started = await w.shell.exec({ script: 'attend first | attend second', timeoutMs: 20 })
    expect(started.status === 'running' && started.runningHint).toBe('second hint')
    second.resolve()
    await waitFor(() => ended)
    const remaining = await w.shell.status({ commandId: started.commandId })
    expect(remaining.status === 'running' && remaining.runningHint).toBe('first hint')
    const stopped = await w.shell.abort({ commandId: started.commandId })
    expect(stopped.status).toBe('aborted')
    expect('runningHint' in stopped).toBe(false)
  } finally { await w.dispose() }
})

test('runtime leaf hints survive the manifest and clear when the caller aborts stdin', async () => {
  const w = await shellFor([{
    name: 'attend', summary: 'A runtime command.', kind: 'runtime', runningHint: 'runtime hint',
    module: runtimeModule('export default async function(ctx) { for await (const chunk of ctx.stdin) await ctx.stdout(chunk); return { exitCode: 0 } }'),
  }])
  try {
    const controller = new AbortController()
    const started = await w.shell.exec({ script: 'attend', timeoutMs: 20, signal: controller.signal })
    expect(started.status === 'running' && started.runningHint).toBe('runtime hint')
    const written = await w.shell.write({ commandId: started.commandId, stdin: 'steer' })
    expect(written.status === 'running' && written.runningHint).toBe('runtime hint')
    controller.abort()
    const stopped = await w.shell.abort({ commandId: started.commandId })
    expect(stopped.status).toBe('aborted')
    expect('runningHint' in stopped).toBe(false)
  } finally { await w.dispose() }
})
