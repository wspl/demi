import { expect, test } from 'bun:test'
import { deferred, delay } from '@demicodes/utils'
import { World } from './world'
import { model } from './driver'
import { FakeProvisioner } from './fake-provisioner'
import type { ManagedOperation } from '../../storage/control'

test(
  'external reset preserves Cloud files and device identity, announces to the model, and is idempotent',
  async () => {
    const fake = new FakeProvisioner()
    const world = await World.create(
      { managedHosts: { provisioner: fake, config: { sweepMs: 60_000 } } }
    )
    try {
      const driver = await world.conversation('cloud')
      const written = await driver.turn({ model: [
          model.shell('write', 'echo retained > note'),
          model.say('written')
        ] })
      expect(written.received[0]).toContain('exitCode: 0')
      const before = await world.api<{ device: { id: string } }>('/api/cloud')
      const operationId = crypto.randomUUID()
      const requests = await Promise.all(
        Array.from(
          { length: 3 },
          () => world.api<{ operation: ManagedOperation }>(
            '/api/cloud/reset',
            { operationId }
          )
        )
      )
      expect(requests.map(result => result.operation.id))
        .toEqual([operationId, operationId, operationId])
      const deadline = Date.now() + 60_000
      while ((await world.api<{ operation: ManagedOperation }>('/api/cloud')).operation.phase !== 'ready') {
        if (Date.now() > deadline)
          throw new Error('Cloud reset did not finish')
        await delay(50)
      }
      expect(
        (await world.api<{ device: { id: string } }>('/api/cloud')).device.id
      )
        .toBe(before.device.id)
      const next = await driver.turn({ model: [
          model.shell('read', 'cat note'),
          model.say('read')
        ] })
      expect(next.received[0]).toContain('retained')
      expect(JSON.stringify(next.requests[0]!.items))
        .toContain(`[Cloud reset ${operationId}]`)
      await world.api('/api/cloud/reset', { operationId })
      expect(fake.calls.filter(call => call.startsWith('reset:')))
        .toHaveLength(1)
    } finally {
      await world.close()
    }
  },
  45_000
)

test(
  'a failed reset reports failure and retries the same operation without losing home',
  async () => {
    class FailOnce extends FakeProvisioner {
      failed = false
      override async reset(
        ...args: Parameters<FakeProvisioner['reset']>
      ): Promise<void> {
        if (!this.failed) {
          this.failed = true;
          throw new Error('image publication unavailable')
        }
        await super.reset(...args)
      }
    }
    const fake = new FailOnce()
    const world = await World.create(
      { managedHosts: { provisioner: fake, config: { sweepMs: 60_000 } } }
    )
    try {
      const driver = await world.conversation('cloud')
      const written = await driver.turn({ model: [
          model.shell('write', 'printf retained > note'),
          model.say('written')
        ] })
      expect(written.received[0]).toContain('exitCode: 0')
      const operationId = crypto.randomUUID()
      const waitForPhase = async (phase: ManagedOperation['phase']) => {
        const deadline = Date.now() + 15_000
        while (true) {
          const state = await world.api<{ operation: ManagedOperation }>(
            '/api/cloud'
          )
          if (state.operation.phase === phase)
            return state.operation
          if (Date.now() > deadline)
            throw new Error(`Reset did not reach ${phase}`)
          await delay(20)
        }
      }
      await world.api('/api/cloud/reset', { operationId })
      expect((await waitForPhase('failed')).error)
        .toContain('image publication unavailable')
      await world.api('/api/cloud/reset', { operationId })
      expect((await waitForPhase('ready')).id).toBe(operationId)
      expect(await driver.readFile('note')).toBe('retained')
    } finally {
      await world.close()
    }
  },
  45_000
)

test(
  'a reset leaves a conversation that only has Cloud attached running, and holds a Cloud conversation until it ends',
  async () => {
    const rebuilding = deferred<void>()
    const proceed = deferred<void>()
    class HeldReset extends FakeProvisioner {
      override async reset(
        ...args: Parameters<FakeProvisioner['reset']>
      ): Promise<void> {
        rebuilding.resolve()
        await proceed.promise
        await super.reset(...args)
      }
    }
    const world = await World.create(
      { managedHosts: { provisioner: new HeldReset(), config: { sweepMs: 60_000 } } }
    )
    try {
      await world.pair('alpha')
      const cloud = await world.conversation('cloud')
      await cloud.turn({ model: [model.shell('write', 'echo kept > note'), model.say('written')] })
      // Leaving Cloud for the paired device keeps Cloud attached.
      const local = await world.conversation('cloud')
      await local.turn({ model: [model.shell('touch', 'true'), model.say('touched')] })
      await local.switchTo('runner:alpha')
      const cloudDevice = await world.api<{ device: { id: string } }>('/api/cloud')
      const attached = await world.api<{ hosts: Array<{ deviceId: string }> }>(`/api/conversations/${local.id}/hosts`)
      expect(attached.hosts.map(host => host.deviceId)).toEqual([cloudDevice.device.id])

      await world.api('/api/cloud/reset', { operationId: crypto.randomUUID() })
      await rebuilding.promise

      // The local conversation is opened, written to and answered mid-reset.
      await local.detach()
      await local.attach()
      const during = await local.turn({ model: [model.shell('local', 'echo on-alpha'), model.say('done')] })
      expect(during.received[0]).toContain('on-alpha')

      // The Cloud conversation waits instead of failing, and opens by itself.
      await cloud.detach()
      let opened = false
      const opening = cloud.attach().then(() => { opened = true })
      await delay(300)
      expect(opened).toBe(false)
      proceed.resolve()
      await opening
      const after = await cloud.turn({ model: [model.shell('read', 'cat note'), model.say('read')] })
      expect(after.received[0]).toContain('kept')
    } finally {
      proceed.resolve()
      await world.close()
    }
  },
  60_000
)
