import { expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { FakeProvisioner } from './fake-provisioner'
import { World } from './world'
import { model } from './driver'

test(
  'concurrent Cloud first use joins one boot; idle saves once and the next command wakes it',
  async () => {
    const fake = new FakeProvisioner()
    const world = await World.create({ managedHosts: {
        provisioner: fake,
        config: {
          idleMs: 250,
          sweepMs: 50,
          checkpointIntervalMs: 60_000,
          bootTimeoutMs: 15_000
        }
      } })
    try {
      const before = await world.api<{
        state: string;
        device: null
      }>('/api/cloud')
      expect(before.state).toBe('unallocated')
      expect(fake.calls).toEqual([])
      const [a, b] = await Promise.all([
        world.conversation('cloud'),
        world.conversation('cloud')
      ])
      const turns = await Promise.all(
        [a, b].map(
          (driver, i) => driver.turn(
            { model: [
                model.shell(`t${i}`, `echo ${i} > note`),
                model.say('done')
              ] }
          )
        )
      )
      for (const turn of turns)
        expect(turn.received[0]).toContain('exitCode: 0')
      expect(fake.guests.size).toBe(1)
      const id = [...fake.guests.keys()][0]!
      expect(fake.calls.filter(call => call === `wake:${id}`)).toHaveLength(1)
      expect(await a.readFile('note')).toBe('0\n')
      expect(await b.readFile('note')).toBe('1\n')
      await waitFor(
        () => fake.calls.includes(`hibernate:${id}`) && !fake.running(id),
        () => fake.calls.join(','),
        { timeoutMs: 10_000 }
      )
      const next = await a.turn({ model: [
          model.shell('next', 'cat note'),
          model.say('awake')
        ] })
      expect(next.received[0]).toContain('0')
      expect(fake.calls.filter(call => call === `wake:${id}`)).toHaveLength(2)
    } finally {
      await world.close()
    }
  },
  45_000
)
