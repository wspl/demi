import { expect, test } from 'bun:test'
import { formatReset, formatSpan } from '../useRelativeTime'

// When a usage window resets, as Settings and the context card both say it
// (`usage-and-quota.md` § What a snapshot says), against a fixed clock in the
// local time zone: Sunday, October 11, 2026, 12:22 PM. Each row of the table,
// and each side of every boundary between two rows.
const now = new Date(2026, 9, 11, 12, 22)

const SECOND = 1_000
const MINUTE = 60 * SECOND
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

/** The moment `ms` from now. */
function inMs(ms: number): string {
  return new Date(now.getTime() + ms).toISOString()
}

/** A moment on the local clock. */
function at(...parts: [number, number, number, number, number, number?, number?]): string {
  return new Date(...parts).toISOString()
}

test.each([
  { scenario: 'the clock has passed', at: inMs(-30 * SECOND), wording: 'Resets now' },
  { scenario: 'the clock reaches', at: inMs(0), wording: 'Resets now' },
  { scenario: 'within a minute', at: inMs(SECOND), wording: 'Resets in <1m' },
  { scenario: '59 s ahead', at: inMs(59 * SECOND), wording: 'Resets in <1m' },
  { scenario: '60 s ahead', at: inMs(MINUTE), wording: 'Resets in 1m' },
  { scenario: 'within the hour, counted down', at: inMs(42 * MINUTE + 59 * SECOND), wording: 'Resets in 42m' },
  { scenario: '59 m 59 s ahead', at: inMs(59 * MINUTE + 59 * SECOND), wording: 'Resets in 59m' },
  { scenario: 'an hour ahead', at: inMs(HOUR), wording: 'Resets in 1h' },
  { scenario: 'within 24 hours', at: inMs(3 * HOUR + 42 * MINUTE), wording: 'Resets in 3h 42m' },
  { scenario: 'on the hour', at: inMs(3 * HOUR), wording: 'Resets in 3h' },
  { scenario: '23 h 59 m ahead', at: inMs(23 * HOUR + 59 * MINUTE), wording: 'Resets in 23h 59m' },
  { scenario: '24 h ahead', at: inMs(DAY), wording: 'Resets Mon 12:22 PM' },
  { scenario: 'within 7 days', at: at(2026, 9, 17, 16, 0), wording: 'Resets Sat 4:00 PM' },
  { scenario: 'a moment just before the minute', at: at(2026, 9, 17, 15, 59, 59, 600), wording: 'Resets Sat 4:00 PM' },
  { scenario: '6 d 23 h ahead', at: inMs(6 * DAY + 23 * HOUR), wording: 'Resets Sun 11:22 AM' },
  { scenario: '7 d ahead', at: inMs(7 * DAY), wording: 'Resets Oct 18' },
  { scenario: 'later this year', at: at(2026, 9, 24, 16, 0), wording: 'Resets Oct 24' },
  { scenario: 'on the last day of the year', at: at(2026, 11, 31, 23, 0), wording: 'Resets Dec 31' },
  { scenario: 'in another year', at: at(2027, 0, 3, 9, 5), wording: 'Resets Jan 3, 2027' },
  { scenario: 'a moment rounding into another year', at: at(2026, 11, 31, 23, 59, 59, 600), wording: 'Resets Jan 1, 2027' },
])('a reset $scenario reads "$wording"', ({ at, wording }) => {
  expect(formatReset(at, now)).toBe(wording)
})

// The duration words the reset times share with the time a subagent has run.
test.each([
  { scenario: 'under a minute', ms: 42 * SECOND + 900, wording: '42s' },
  { scenario: 'on the minute', ms: 3 * MINUTE, wording: '3m' },
  { scenario: 'minutes and seconds', ms: 16 * MINUTE + 48 * SECOND, wording: '16m 48s' },
  { scenario: 'an hour and seconds', ms: HOUR + 59 * SECOND, wording: '1h' },
  { scenario: 'hours and minutes', ms: HOUR + 5 * MINUTE + 30 * SECOND, wording: '1h 5m' },
  { scenario: 'more than a day', ms: DAY + 3 * HOUR, wording: '27h' },
  { scenario: 'a clock set back', ms: -5 * SECOND, wording: '0s' },
])('a span of $scenario reads "$wording"', ({ ms, wording }) => {
  expect(formatSpan(ms)).toBe(wording)
})
