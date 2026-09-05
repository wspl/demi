import { mkdtemp, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { HostlessEnvironment } from '@demicodes/host-virtual'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { ActivityGate, deferred, waitFor } from '@demicodes/utils'
import { UpgradingShell } from '../conversation/upgrading-shell'

test('cutover waits for an observed-running script to finish its last file write', async () => {
  const home = await mkdtemp(join(tmpdir(), 'demi-cutover-drain-'))
  const host = new LocalHost(home)
  const finish = deferred<void>()
  const gate = new ActivityGate()
  const local = new HostlessEnvironment({
    host, home, namespace: [home], identity: { user: 'test', group: 'test' },
    roots: new Map([['hold', () => []]]),
    dispatch: async () => {
      await finish.promise
      await host.fs.writeFile('late.txt', new TextEncoder().encode('late'), { cwd: home })
      return 0
    },
  })
  let requested = false
  const snapshot: { value: string | null } = { value: null }
  const shell = new UpgradingShell(local, async () => {
    requested = true
    const release = await gate.reserve()
    try {
      snapshot.value = await readFile(join(home, 'late.txt'), 'utf8')
      throw new Error('provision refused')
    } finally { release() }
  }, home, gate, async () => {})
  try {
    const running = await shell.exec({ script: 'hold', ephemeral: true, timeoutMs: 1 })
    expect(running.status).toBe('running')
    const upgrade = shell.exec({ script: 'uname', ephemeral: true }).catch(error => error as Error)
    await waitFor(() => requested)
    expect(snapshot.value).toBeNull()
    finish.resolve()
    expect(await upgrade).toBeInstanceOf(Error)
    expect(snapshot.value).toBe('late')
    expect(gate.active).toBe(false)
  } finally {
    finish.resolve()
    await shell.disposeAllShells()
  }
})
