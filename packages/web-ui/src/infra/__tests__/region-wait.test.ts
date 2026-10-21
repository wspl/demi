import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { effectScope, shallowRef, watchEffect } from 'vue'
import { REGION_WAIT_MS, pinRegionForms, regionForm, regionsChanged, resetRegionWaits, type RegionForm } from '../region-wait'

// A region that needs things which are not there waits for them, for 300 ms at
// most in all, and draws again once when it is final. Fake timers; no I/O.
beforeEach(() => {
  jest.useFakeTimers()
  resetRegionWaits()
})
afterEach(() => {
  pinRegionForms(null)
  jest.useRealTimers()
})

/** A region shown: each draw it makes, as the form it was told. */
function show(key: string, ready: () => boolean): { draws: RegionForm[]; remove: () => void } {
  const draws: RegionForm[] = []
  const scope = effectScope()
  scope.run(() => watchEffect(() => draws.push(regionForm(key, ready)), { flush: 'sync' }))
  return { draws, remove: () => scope.stop() }
}

test('a region whose things arrive within 300 ms draws once, final', () => {
  const arrived = shallowRef(false)
  const region = show('code:ts', () => arrived.value)
  expect(region.draws).toEqual(['held'])
  jest.advanceTimersByTime(REGION_WAIT_MS - 1)
  arrived.value = true
  regionsChanged()
  expect(region.draws.at(-1)).toBe('final')
  // The clock's end changes nothing once it is final.
  jest.advanceTimersByTime(REGION_WAIT_MS)
  expect(new Set(region.draws)).toEqual(new Set(['held', 'final']))
  region.remove()
})

test('a region later than 300 ms draws plain at 300 ms, which stays until it is final', () => {
  const arrived = shallowRef(false)
  const region = show('math', () => arrived.value)
  jest.advanceTimersByTime(REGION_WAIT_MS)
  expect(region.draws.at(-1)).toBe('plain')
  arrived.value = true
  regionsChanged()
  expect(region.draws.at(-1)).toBe('final')
  expect(region.draws.filter((form) => form === 'final').length).toBeGreaterThan(0)
  expect(region.draws.indexOf('final')).toBeGreaterThan(region.draws.indexOf('plain'))
  region.remove()
})

test('a region that waits for two things in turn waits 300 ms in all, and its plain form does not go away between them', () => {
  const list = shallowRef(false)
  const picture = shallowRef(false)
  const region = show('icon:file:a.ts', () => list.value && picture.value)
  // The first thing arrives within the 300 ms; the second does not.
  jest.advanceTimersByTime(200)
  list.value = true
  regionsChanged()
  expect(region.draws.at(-1)).toBe('held')
  jest.advanceTimersByTime(100)
  expect(region.draws.at(-1)).toBe('plain')
  // The second arrives much later: no second wait, and no hidden form between.
  jest.advanceTimersByTime(5000)
  expect(region.draws.at(-1)).toBe('plain')
  picture.value = true
  regionsChanged()
  expect(region.draws.at(-1)).toBe('final')
  expect(region.draws.slice(region.draws.indexOf('plain'))).not.toContain('held')
  region.remove()
})

test('a region shown while another waits for the same key waits out what is left of the 300 ms', () => {
  const ready = () => false
  const first = show('math', ready)
  jest.advanceTimersByTime(200)
  const second = show('math', ready)
  expect(second.draws).toEqual(['held'])
  jest.advanceTimersByTime(100)
  expect([first.draws.at(-1), second.draws.at(-1)]).toEqual(['plain', 'plain'])
  first.remove()
  second.remove()
})

test('pinned, every region draws in the form it is pinned to, and in its own again once unpinned', () => {
  const arrived = shallowRef(true)
  pinRegionForms('held')
  const region = show('code:ts', () => arrived.value)
  expect(region.draws.at(-1)).toBe('held')
  pinRegionForms('plain')
  expect(region.draws.at(-1)).toBe('plain')
  pinRegionForms(null)
  expect(region.draws.at(-1)).toBe('final')
  region.remove()
})
