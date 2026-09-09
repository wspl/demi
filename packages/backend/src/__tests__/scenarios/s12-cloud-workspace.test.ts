import { expect, test } from 'bun:test'
import { World } from './world'
import { model } from './driver'

test(
  'Cloud projects share one user machine and can read each other; deleting a project preserves files',
  async () => {
    const world = await World.create()
    try {
      type Workspace = {
        id: string;
        deviceId: string;
        path: string
      }
      const [a, b] = await Promise.all(
        ['first', 'second'].map(
          name => world.api<{ workspace: Workspace }>(
            '/api/workspaces',
            { cloud: true, name }
          )
        )
      )
      expect(a.workspace.deviceId).toBe(b.workspace.deviceId)
      expect(a.workspace.path).not.toBe(b.workspace.path)
      const first = await world.conversation('cloud')
      const second = await world.conversation('cloud')
      for (const [driver, workspace] of [
        [first, a.workspace],
        [second, b.workspace]
      ] as const) {
        await world.api(
          `/api/conversations/${driver.id}`,
          { target: { kind: 'workspace', workspaceId: workspace.id } },
          'PATCH'
        )
      }
      const wrote = await first.turn({ model: [
          model.shell('a', 'printf shared > note'),
          model.say('written')
        ] })
      expect(wrote.received[0]).toContain('exitCode: 0')
      const read = await second.turn({ model: [
          model.shell('b', `cat '${a.workspace.path}/note'`),
          model.say('read')
        ] })
      expect(read.received[0]).toContain('shared')
      await expect(
        world.api(`/api/workspaces/${a.workspace.id}`, undefined, 'DELETE')
      ).rejects.toThrow('409')
      await world.api(
        `/api/conversations/${first.id}`,
        { target: { kind: 'cloud' } },
        'PATCH'
      )
      await world.api(`/api/workspaces/${a.workspace.id}`, undefined, 'DELETE')
      const retained = await second.turn({ model: [
          model.shell('c', `cat '${a.workspace.path}/note'`),
          model.say('retained')
        ] })
      expect(retained.received[0]).toContain('shared')
    } finally {
      await world.close()
    }
  },
  60_000
)

test(
  'Cloud creation is explicitly unavailable without a provisioner',
  async () => {
    const world = await World.create({ managedHosts: null })
    try {
      await expect(world.api('/api/workspaces', { cloud: true, name: 'unavailable' })).rejects.toThrow(
        '409'
      )
    }
    finally {
      await world.close()
    }
  }
)
