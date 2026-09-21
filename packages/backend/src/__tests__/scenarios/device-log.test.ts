import { expect, test } from 'bun:test'
import { delay, waitFor } from '@demicodes/utils'
import { FakeProvisioner } from '../../testing/fake-provisioner'
import { World } from './world'
import { model } from './driver'

// The device log route (`web-api.md` § Device log) over real runners: a
// paired device and the user's Cloud answer alike, and a read wakes nothing.

interface LogPage {
  lines: Array<{ at: string; source: string; conversationId?: string; text: string }>
  next: number
}

/**
 * The runner's writer puts a queued line in the files a moment after it was
 * written, and a read waits for no queued line: ask until `text` is there.
 */
async function logWith(world: World, path: string, text: string): Promise<LogPage> {
  let page = await world.api<LogPage>(path)
  for (let tries = 0; tries < 500 && !page.lines.some(line => line.text === text); tries += 1) {
    await delay(10)
    page = await world.api<LogPage>(path)
  }
  expect(page.lines.map(line => line.text)).toContain(text)
  return page
}

test(
  'a paired device answers its log by cursor, limit and source; offline it answers 409',
  async () => {
    const world = await World.create()
    try {
      const device = await world.pair('laptop')
      const path = `/api/devices/${device.deviceId}/log`
      const tail = await logWith(world, path, 'online')
      expect(tail.lines.every(line => line.source === 'runner')).toBe(true)
      expect(tail.lines.every(line => Number.isFinite(Date.parse(line.at)))).toBe(true)
      expect(tail.lines.some(line => line.text === 'waiting to be paired')).toBe(true)
      // The pairing code is for the console alone.
      expect(tail.lines.some(line => line.text.includes(device.runner.codes[0]!))).toBe(false)

      expect(await world.api<LogPage>(`${path}?since=${tail.next}`)).toEqual({ lines: [], next: tail.next })
      const first = await world.api<LogPage>(`${path}?since=0&limit=1`)
      expect(first.lines).toEqual([tail.lines[0]!])
      const second = await world.api<LogPage>(`${path}?since=${first.next}&limit=1`)
      expect(second.lines).toEqual([tail.lines[1]!])
      const other = await world.api<LogPage>(`${path}?since=0&source=${encodeURIComponent('service:none')}`)
      expect(other).toEqual({ lines: [], next: tail.next })

      for (const query of ['limit=0', 'limit=1001', 'limit=many', 'since=-1', 'since=1.5', 'source=']) {
        const refused = await world.backend.session.fetch(`${path}?${query}`)
        expect(refused.status).toBe(400)
        expect(await refused.json()).toMatchObject({ code: 'invalid_query' })
      }
      const unknown = await world.backend.session.fetch('/api/devices/none/log')
      expect(unknown.status).toBe(404)

      await device.runner.stop()
      let online = true
      for (let tries = 0; tries < 500 && online; tries += 1) {
        const listed = await world.api<{ devices: Array<{ id: string; online: boolean }> }>('/api/devices')
        online = listed.devices.some(row => row.id === device.deviceId && row.online)
        if (online)
          await delay(10)
      }
      expect(online).toBe(false)
      const offline = await world.backend.session.fetch(path)
      expect(offline.status).toBe(409)
      expect(await offline.json()).toMatchObject({ code: 'device_offline' })
    } finally {
      await world.close()
    }
  },
  30_000
)

test(
  'the Cloud answers its log while it runs; stopped it answers 409 without waking, and the log is there after the wake',
  async () => {
    const fake = new FakeProvisioner()
    const world = await World.create({
      lifecycle: { idleMs: 250 },
      managedHosts: {
        provisioner: fake,
        config: {
          sweepMs: 50,
          checkpointIntervalMs: 60_000,
          bootTimeoutMs: 15_000
        }
      } })
    try {
      const driver = await world.conversation('cloud')
      await driver.turn({ model: [model.shell('first', 'true'), model.say('done')] })
      const cloud = await world.api<{ device: { id: string } }>('/api/cloud')
      const id = cloud.device.id
      const path = `/api/devices/${id}/log`
      const running = await logWith(world, path, 'online')

      await waitFor(
        () => fake.calls.includes(`hibernate:${id}`) && !fake.running(id),
        () => fake.calls.join(','),
        { timeoutMs: 10_000 }
      )
      const stopped = await world.backend.session.fetch(path)
      expect(stopped.status).toBe(409)
      expect(await stopped.json()).toMatchObject({ code: 'device_offline' })
      expect(fake.calls.filter(call => call === `wake:${id}`)).toHaveLength(1)

      await driver.turn({ model: [model.shell('second', 'true'), model.say('awake')] })
      expect(fake.calls.filter(call => call === `wake:${id}`)).toHaveLength(2)
      const woken = await logWith(world, `${path}?since=${running.next}`, 'online')
      expect(woken.lines.some(line => line.text === 'runner stopped')).toBe(true)
      expect(woken.lines.some(line => /^runner \S+ started$/.test(line.text))).toBe(true)
    } finally {
      await world.close()
    }
  },
  45_000
)
