import { expect, test } from 'bun:test'
import { fieldMoment, shortMoment, zonedMoment } from '../words'

// An automation's times are shown and taken in its own time zone, not the
// browser's: the editor's Once field reads and writes them through these. A
// skipped time is the first minute after the gap and a repeated one its first
// occurrence, as the scheduler reads them. Pure functions; milliseconds.

test('a Once moment is shown and taken by the clocks of the automation’s zone, whatever the browser’s', () => {
  // 08:00 in Tokyo, New York and Shanghai: three different moments.
  expect(zonedMoment(2026, 10, 18, 8, 0, 'Asia/Tokyo').toISOString()).toBe('2026-10-17T23:00:00.000Z')
  expect(zonedMoment(2026, 10, 18, 8, 0, 'America/New_York').toISOString()).toBe('2026-10-18T12:00:00.000Z')
  expect(zonedMoment(2026, 10, 18, 8, 0, 'Asia/Shanghai').toISOString()).toBe('2026-10-18T00:00:00.000Z')
  // And back: the field shows what was taken, by the same clocks.
  expect(fieldMoment('2026-10-18T12:00:00Z', 'America/New_York')).toBe('2026-10-18 08:00')
  expect(fieldMoment('2026-10-17T23:00:00Z', 'Asia/Tokyo')).toBe('2026-10-18 08:00')
  expect(shortMoment('2026-10-17T23:00:00Z', 'Asia/Tokyo')).toMatch(/18.* 08:00$/)
  expect(shortMoment('2026-10-17T23:00:00Z', 'America/New_York')).toMatch(/17.* 19:00$/)
})

test('a time the clocks skip is the first minute after the gap, and a repeated one its first occurrence', () => {
  // 02:30 does not exist in New York on 8 March 2026: 03:00 EDT.
  expect(zonedMoment(2026, 3, 8, 2, 30, 'America/New_York').toISOString()).toBe('2026-03-08T07:00:00.000Z')
  // 01:30 happens twice on 1 November 2026: the first, EDT.
  expect(zonedMoment(2026, 11, 1, 1, 30, 'America/New_York').toISOString()).toBe('2026-11-01T05:30:00.000Z')
})
