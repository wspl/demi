import { expect, test } from 'bun:test'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { migrate, CONTROL_MIGRATIONS } from '../storage/migrations'

test(
  'Cloud and device target switches keep files in place and attach the departed host',
  async () => {
    const world = await World.create({ runners: ['laptop'] })
    try {
      const driver = await world.conversation('cloud')
      await driver.turn({ model: [
          model.shell('write', 'echo cloud-file > note'),
          model.say('written')
        ] })
      const cloud = await world.api<{ device: { id: string } }>('/api/cloud')
      await driver.switchTo('runner:laptop')
      const switched = await driver.turn({ model: [
          model.shell('current', 'demi host current; demi host list'),
          model.say('switched')
        ] })
      expect(switched.received[0]).toContain('laptop')
      const hosts = await world.api<{ hosts: { deviceId: string }[] }>(
        `/api/conversations/${driver.id}/hosts`
      )
      expect(hosts.hosts.map(host => host.deviceId)).toContain(cloud.device.id)
      const pulled = await driver.turn({ model: [
          model.shell('pull', 'demi host shell --host Cloud "cat note"'),
          model.say('read')
        ] })
      expect(pulled.received[0]).toContain('cloud-file')
      await driver.switchTo('cloud')
      const returned = await driver.turn({ model: [
          model.shell('return', 'cat note'),
          model.say('returned')
        ] })
      expect(returned.received[0]).toContain('cloud-file')
      const attached = await world.api<{ hosts: { deviceId: string }[] }>(
        `/api/conversations/${driver.id}/hosts`
      )
      expect(attached.hosts.map(host => host.deviceId))
        .toContain(world.device('laptop').deviceId)
    } finally {
      await world.close()
    }
  },
  45_000
)

test(
  'target boundary validates discriminators, owned references, and archived conversation switches',
  async () => {
    const world = await World.create({ runners: ['laptop'] })
    try {
      const driver = await world.conversation('cloud')
      for (const target of [
        { kind: 'hostless' },
        { kind: 'device' },
        { kind: 'cloud', deviceId: 'extra' }
      ]) {
        await expect(
          world.api(`/api/conversations/${driver.id}`, { target }, 'PATCH')
        ).rejects.toThrow('400')
      }
      await expect(
        world.api(
          `/api/conversations/${driver.id}`,
          { target: { kind: 'workspace', workspaceId: 'missing' } },
          'PATCH'
        )
      ).rejects.toThrow('404')
      await world.api(`/api/conversations/${driver.id}`, { target: {
          kind: 'device',
          deviceId: world.device('laptop').deviceId,
          path: world.device('laptop').home
        } }, 'PATCH')
      await world.api(
        `/api/conversations/${driver.id}`,
        { archived: true },
        'PATCH'
      )
      await expect(
        world.api(
          `/api/conversations/${driver.id}`,
          { target: { kind: 'cloud' } },
          'PATCH'
        )
      ).rejects.toThrow('409')
    } finally {
      await world.close()
    }
  },
  30_000
)

test(
  'target compare-and-swap admits one winner and preserves the losing switch announcement',
  async () => {
    const db = openSqliteDatabase(':memory:')
    migrate(db, CONTROL_MIGRATIONS)
    const control = new LocalControlService(db)
    try {
      const user = (await control.createMaster(
        { email: 'test@example.test', passwordHash: '' }
      ))!
      const conversation = await control.createConversation(user.id)
      const device = await control.createDevice({
        userId: user.id,
        name: 'device',
        platform: 'test',
        tokenHash: 'token'
      })
      const from = {
        kind: 'cloud' as const,
        deviceId: null,
        path: `/home/demi/sessions/${conversation.id}`
      }
      const to = {
        kind: 'device' as const,
        deviceId: device.id,
        path: '/first'
      }
      const switchTo = (path: string) => control.switchConversationTarget(
        conversation.id,
        { kind: 'cloud' },
        { ...to, path },
        { from, to: { ...to, path } },
        { departed: null, arrivingDeviceId: device.id }
      )
      expect(await Promise.all([
        switchTo('/first'),
        switchTo('/second')
      ])).toEqual([true, false])
      expect(
        (await control.getConversation(conversation.id))?.lastSwitch?.to.path
      )
        .toBe('/first')
    } finally {
      db.close()
    }
  }
)
