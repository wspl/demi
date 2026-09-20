import { expect, test } from 'bun:test'
import { World } from './world'

interface CliState {
  newest: { version?: string; error?: string }
  held: string | null
  install: { state: string; message?: string } | null
  machines: Array<{ name: string; versions: string[] | null }>
}

test(
  'a Claude Code entry reads its CLI from the machines, installs on Cloud after an account is added, and says why an install failed',
  async () => {
    const world = await World.create({ runners: ['laptop'] })
    try {
      const { provider } = await world.api<{ provider: { id: string } }>(
        '/api/providers/setup-token',
        { token: 'sk-ant-oat01-fixture', label: 'Claude' }
      )
      const path = `/api/providers/${provider.id}/cli`
      // The paired laptop is asked through the demi.claude package; it has nothing yet.
      const first = await world.api<CliState>(path)
      expect(first.newest).toEqual({ version: '9.9.9' })
      expect(first.held).toBeNull()
      expect(first.machines.find(machine => machine.name === 'laptop')?.versions).toEqual([])

      // Adding the account started the install on the user's Cloud, which the
      // fake distribution has no build for: the account stays, the reason shows.
      let state: CliState = first
      for (let waited = 0; state.install?.state !== 'failed' && waited < 30_000; waited += 100) {
        await Bun.sleep(100)
        state = await world.api<CliState>(path)
      }
      expect(state.install?.message).toContain('Claude Code 9.9.9 could not be installed')
      expect((await world.api<{ accounts: unknown[] }>(`/api/providers/${provider.id}/accounts`)).accounts)
        .toHaveLength(1)

      // A version the vendor does not publish cannot be held; one it does can.
      await expect(world.api(path, { held: '1.2.3' }, 'PUT')).rejects.toThrow('unknown_version')
      await world.api(path, { held: '9.9.9' }, 'PUT')
      expect((await world.api<CliState>(path)).held).toBe('9.9.9')
      await world.api(path, { held: null }, 'PUT')
      expect((await world.api<CliState>(path)).held).toBeNull()
    } finally {
      await world.close()
    }
  },
  60_000
)
