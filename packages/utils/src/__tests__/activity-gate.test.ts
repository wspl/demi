import { expect, test } from 'bun:test'
import { ActivityGate } from '../activity-gate'

test('reservation purpose is observable while draining and is cleared on release', async () => {
  const gate = new ActivityGate()
  const purposes: Array<'idle' | 'forced' | undefined> = []
  const unsubscribe = gate.subscribe(() => { purposes.push(gate.reservationPurpose) })
  const idle = gate.tryReserve('idle')!
  expect(gate.reservationPurpose).toBe('idle')
  expect(gate.holdsReservation(idle)).toBe(true)
  idle()
  const lease = gate.tryEnter()!
  const resetting = gate.reserve('forced')
  expect(gate.reservationPurpose).toBe('forced')
  lease()
  const reset = await resetting
  expect(gate.holdsReservation(reset)).toBe(true)
  reset()
  expect(gate.reservationPurpose).toBeUndefined()
  expect(purposes).toEqual(['idle', undefined, undefined, 'forced', 'forced', undefined])
  unsubscribe()
})

test(
  'exclusive admission drains current work and queues new work until release',
  async () => {
    const gate = new ActivityGate()
    const first = await gate.enter()
    const second = await gate.enter()
    expect(gate.tryReserve('forced')).toBeNull()
    const events: string[] = []
    const writer = gate.reserve('forced').then(release => {
      events.push('writer');
      return release
    })
    const entrant = gate.enter().then(release => {
      events.push('entrant');
      return release
    })
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
  }
)

test(
  'canceling a draining reservation opens admission without discarding active work',
  async () => {
    const gate = new ActivityGate()
    const leave = await gate.enter()
    const abort = new AbortController()
    const writer = gate.reserve('forced', abort.signal)
    writer.catch(() => {})
    const entrant = gate.enter()
    abort.abort()
    await expect(writer).rejects.toThrow()
    const leaveEntrant = await entrant
    leaveEntrant()
    expect(gate.active).toBe(true)
    leave()
    const release = gate.tryReserve('forced')
    expect(release).not.toBeNull()
    release!()
  }
)

test(
  'an idle reservation excludes entrants and canceled waiters never acquire admission',
  async () => {
    const gate = new ActivityGate()
    const release = gate.tryReserve('idle')!
    expect(gate.tryReserve('idle')).toBeNull()
    const abort = new AbortController()
    const entering = gate.enter(abort.signal)
    entering.catch(() => {})
    abort.abort()
    await expect(entering).rejects.toThrow()
    release()
    expect(gate.active).toBe(false)
    const reserveAbort = new AbortController()
    reserveAbort.abort()
    await expect(gate.reserve('forced', reserveAbort.signal)).rejects.toThrow()
    const final = gate.tryReserve('idle')
    expect(final).not.toBeNull()
    final?.()
  }
)
