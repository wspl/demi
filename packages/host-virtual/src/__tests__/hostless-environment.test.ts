import { describe, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { ShellExecInput } from '@demicodes/shell'
import { collectBytes, delay, emptyByteStream } from '@demicodes/utils'
import { HostlessEnvironment, VirtualHost, type HostlessEnvironmentOptions, type VirtualHostOptions } from '../index'
import { LocalHost, scopedFsBackend } from '../testing'

async function world(options: { quota?: VirtualHostOptions['quota']; dispatch?: HostlessEnvironmentOptions['dispatch'] } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'hostless-environment-'))
  const local = new LocalHost(dir, { storeRoot: join(dir, 'store') })
  const home = '/home/demi'
  const host = new VirtualHost({
    backend: scopedFsBackend(join(dir, 'files'), local.fs),
    store: local.store,
    defaultCwd: home,
    quota: options.quota,
  })
  await host.ensureLayout()
  const env = new HostlessEnvironment({
    host,
    roots: new Map(options.dispatch ? [['probe', () => []]] : []),
    dispatch: options.dispatch ?? (async () => { throw new Error('Unexpected root command') }),
    home,
    namespace: [home],
    identity: { user: 'demi', group: 'demi' },
    initialEnv: { PATH: '/usr/bin:/bin', LANG: 'C' },
  })
  return {
    env,
    exec: (script: string, options: Omit<ShellExecInput, 'script'> = {}) =>
      env.exec({ script, agentSessionId: 'session-1', timeoutMs: 1_000, ...options }),
    dispose: async () => {
      await env.disposeAllShells()
      rmSync(dir, { recursive: true, force: true })
    },
  }
}

describe('hostless shell lifecycle', () => {
  test('the caller signal interrupts a command waiting for stdin and leaves its shell reusable', async () => {
    const w = await world()
    try {
      const controller = new AbortController()
      const running = await w.exec('cat; echo never', { timeoutMs: 10, signal: controller.signal })
      expect(running.status).toBe('running')
      controller.abort()
      await delay(0)
      const aborted = await w.env.status({ commandId: running.commandId })
      expect(aborted.status).toBe('aborted')
      expect(aborted.stdout.tail).toBe('')
      expect(aborted.stderr.tail).toBe('')
      const next = await w.exec('echo reused', { shellId: running.shellId })
      expect(next.status).toBe('exited')
      expect(next.stdout.tail).toBe('reused\n')
    } finally {
      await w.dispose()
    }
  })

  test('shell_abort reports aborted when closing stdin lets the final builtin finish', async () => {
    const w = await world()
    try {
      const running = await w.exec('cat', { timeoutMs: 10 })
      expect(running.status).toBe('running')
      const aborted = await w.env.abort({ commandId: running.commandId })
      expect(aborted.status).toBe('aborted')
      expect(aborted.stderr.tail).toBe('')
    } finally {
      await w.dispose()
    }
  })

  test('cancellation remains aborted when a dispatched command returns success after stdin closes', async () => {
    const w = await world({
      dispatch: async (_root, _argv, io) => {
        await collectBytes(io.stdinStream ?? emptyByteStream())
        return 0
      },
    })
    try {
      const running = await w.exec('probe', { timeoutMs: 10 })
      expect(running.status).toBe('running')
      const aborted = await w.env.abort({ commandId: running.commandId })
      expect(aborted.status).toBe('aborted')
    } finally {
      await w.dispose()
    }
  })

  test('handover carries changed initial variables and omits untouched defaults', async () => {
    const w = await world()
    try {
      await w.exec('PATH=/home/demi/bin:/usr/bin:/bin; CUSTOM=value')
      const handover = w.env.handoverOf({ script: '', agentSessionId: 'session-1' })
      expect(handover).toMatchObject({
        agentSessionId: 'session-1',
        isDefault: true,
        cwd: '/home/demi',
        vars: { PATH: '/home/demi/bin:/usr/bin:/bin', CUSTOM: 'value' },
      })
      expect(w.env.getShell(handover.shellId)?.id).toBe(handover.shellId)
      expect(() => w.env.handoverOf({ script: '', ephemeral: true })).toThrow('no shell to hand over')
      await w.exec('PATH=/usr/bin:/bin')
      expect(w.env.handoverOf({ script: '', agentSessionId: 'session-1' }).vars).toEqual({ CUSTOM: 'value' })
    } finally {
      await w.dispose()
    }
  })
})

describe('hostless builtin streams', () => {
  test('head -c finishes at the requested count without another stdin write', async () => {
    const w = await world()
    try {
      const running = await w.exec('head -c 1; echo done', { timeoutMs: 10 })
      expect(running.status).toBe('running')
      await w.env.write({ commandId: running.commandId, stdin: 'a' })
      await delay(0)
      const done = await w.env.status({ commandId: running.commandId })
      expect(done.status).toBe('exited')
      expect(done.stdout.tail).toBe('adone\n')
    } finally {
      await w.dispose()
    }
  })

  test('head -c 0 never waits for live stdin', async () => {
    const w = await world()
    try {
      const done = await w.exec('head -c 0; echo done')
      expect(done.status).toBe('exited')
      expect(done.stdout.tail).toBe('done\n')
    } finally {
      await w.dispose()
    }
  })

  test('a redirection quota failure runs its || recovery and later statements', async () => {
    const w = await world({ quota: { maxFileBytes: 1 } })
    try {
      const done = await w.exec('echo abc > foo || echo recovery; echo done')
      expect(done.status).toBe('exited')
      if (done.status === 'exited') expect(done.exitCode).toBe(0)
      expect(done.stdout.tail).toBe('recovery\ndone\n')
      expect(done.stderr.tail).toContain('File too large')
    } finally {
      await w.dispose()
    }
  })
})
