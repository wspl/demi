import { expect, test } from 'bun:test'
import { ActivityGate, deferred, delay } from '@demicodes/utils'
import { LifecycleCoordinator } from '../coordinator'

test('maintenance postpones retirement without restarting its elapsed idle interval', async () => {
  const gate = new ActivityGate()
  const coordinator = new LifecycleCoordinator()
  const retired = deferred<void>()
  const maintenance = gate.tryEnter('maintenance')!
  coordinator.idle({
    idleMs: 30,
    demand: () => gate.demandActive,
    eligible: () => !gate.demandActive,
    observe: changed => gate.subscribe(changed),
    reserve: () => {
      const release = gate.tryReserve('idle')
      return release ? { run: async () => { retired.resolve() }, release } : null
    },
  })
  await delay(65)
  const releasedAt = performance.now()
  maintenance()
  await retired.promise
  expect(performance.now() - releasedAt).toBeLessThan(25)
  await coordinator.close()
})

test('new demand resets an idle interval and no admission is reclaimed while active', async () => {
  const gate = new ActivityGate()
  const coordinator = new LifecycleCoordinator()
  let retirements = 0
  coordinator.idle({
    idleMs: 35,
    demand: () => gate.demandActive,
    eligible: () => !gate.demandActive,
    observe: changed => gate.subscribe(changed),
    reserve: () => {
      const release = gate.tryReserve('idle')
      return release ? { run: async () => { retirements++ }, release } : null
    },
  })
  await delay(20)
  const release = await gate.enter()
  await delay(45)
  expect(retirements).toBe(0)
  release()
  await delay(20)
  expect(retirements).toBe(0)
  await delay(35)
  expect(retirements).toBe(1)
  await coordinator.close()
})

test('disposal joins an already unregistered retirement and unrelated resources continue', async () => {
  const coordinator = new LifecycleCoordinator()
  const began = deferred<void>()
  const finish = deferred<void>()
  const other = deferred<void>()
  const remove = coordinator.idle({
    idleMs: 1,
    eligible: () => true,
    observe: () => () => {},
    reserve: () => ({ run: async () => { began.resolve(); await finish.promise }, release: () => {} }),
  })
  coordinator.idle({
    idleMs: 2,
    eligible: () => true,
    observe: () => () => {},
    reserve: () => ({ run: async () => { other.resolve() }, release: () => {} }),
  })
  await began.promise
  remove()
  await other.promise
  let closed = false
  const close = coordinator.close().then(() => { closed = true })
  await delay(5)
  expect(closed).toBe(false)
  finish.resolve()
  await close
  expect(closed).toBe(true)
})

test('a failed cleanup releases its gate and backs off instead of retrying in a loop', async () => {
  const gate = new ActivityGate()
  const errors: string[] = []
  const coordinator = new LifecycleCoordinator(() => performance.now(), message => errors.push(message))
  coordinator.idle({
    idleMs: 50,
    eligible: () => true,
    observe: changed => gate.subscribe(changed),
    reserve: () => {
      const release = gate.tryReserve('idle')
      return release ? { run: async () => { throw new Error('release failed') }, release } : null
    },
  })
  await delay(75)
  expect(errors).toEqual(['release failed'])
  const release = gate.tryEnter()
  expect(release).not.toBeNull()
  await coordinator.close()
  release?.()
})

test('brief demand while an asynchronous reservation waits starts a fresh idle interval', async () => {
  const gate = new ActivityGate()
  const coordinator = new LifecycleCoordinator()
  const reserving = deferred<void>()
  const proceed = deferred<void>()
  const retired = deferred<void>()
  let attempts = 0
  let retirements = 0
  coordinator.idle({
    idleMs: 40,
    demand: () => gate.demandActive,
    eligible: () => !gate.demandActive,
    observe: changed => gate.subscribe(changed),
    reserve: async () => {
      if (attempts++ === 0) {
        reserving.resolve()
        await proceed.promise
      }
      const release = gate.tryReserve('idle')
      return release ? {
        run: async () => {
          retirements++
          retired.resolve()
        },
        release,
      } : null
    },
  })
  try {
    await reserving.promise
    const release = await gate.enter()
    release()
    proceed.resolve()
    await delay(15)
    expect(retirements).toBe(0)
    await retired.promise
    expect(retirements).toBe(1)
  } finally {
    proceed.resolve()
    await coordinator.close()
  }
})
