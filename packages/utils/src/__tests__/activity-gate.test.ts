import { expect, test } from 'bun:test'
import { ActivityGate } from '../activity-gate'

test('exclusive admission drains current work and queues new work until release', async () => {
  const gate = new ActivityGate()
  const first = await gate.enter()
  const second = await gate.enter()
  expect(gate.tryReserve()).toBeNull()
  const events: string[] = []
  const writer = gate.reserve().then(release => { events.push('writer'); return release })
  const entrant = gate.enter().then(release => { events.push('entrant'); return release })
  first()
  first()
  await Promise.resolve()
  expect(events).toEqual([])
  second()
  const releaseWriter = await writer
  expect(events).toEqual(['writer'])
  releaseWriter()
  releaseWriter()
  const releaseEntrant = await entrant
  expect(events).toEqual(['writer', 'entrant'])
  expect(gate.active).toBe(true)
  releaseEntrant()
  expect(gate.active).toBe(false)
})

test('canceling a draining reservation opens admission without discarding active work', async () => {
  const gate = new ActivityGate()
  const leave = await gate.enter()
  const abort = new AbortController()
  const writer = gate.reserve(abort.signal)
  writer.catch(() => {})
  const entrant = gate.enter()
  abort.abort()
  await expect(writer).rejects.toThrow()
  const leaveEntrant = await entrant
  leaveEntrant()
  expect(gate.active).toBe(true)
  leave()
  const release = gate.tryReserve()
  expect(release).not.toBeNull()
  release!()
})

test('an idle reservation excludes entrants and canceled waiters never acquire admission', async () => {
  const gate = new ActivityGate()
  const release = gate.tryReserve()!
  expect(gate.tryReserve()).toBeNull()
  const abort = new AbortController()
  const entering = gate.enter(abort.signal)
  entering.catch(() => {})
  abort.abort()
  await expect(entering).rejects.toThrow()
  release()
  expect(gate.active).toBe(false)
  const reserveAbort = new AbortController()
  reserveAbort.abort()
  await expect(gate.reserve(reserveAbort.signal)).rejects.toThrow()
  expect(gate.tryReserve()).not.toBeNull()
})
