import { expect, test } from 'bun:test'
import { formatReset } from '../useRelativeTime'

// When a usage window resets, as Settings and the context card both say it
// (`usage-and-quota.md`), against a fixed clock in the local time zone:
// Sunday, October 11, 2026, 12:22 PM.
const now = new Date(2026, 9, 11, 12, 22)

/** The moment `minutes` from now. */
function inMinutes(minutes: number): string {
  return new Date(now.getTime() + minutes * 60_000).toISOString()
}

test.each([
  { scenario: 'within the hour', at: inMinutes(12), wording: 'Resets in 12 min' },
  { scenario: 'the last part of a minute counts as one', at: inMinutes(0.25), wording: 'Resets in 1 min' },
  { scenario: 'the clock just passed', at: inMinutes(-0.5), wording: 'Resets in 1 min' },
  { scenario: 'within 24 hours', at: inMinutes(3 * 60 + 38), wording: 'Resets in 3 hr 38 min' },
  { scenario: 'on the hour', at: inMinutes(3 * 60), wording: 'Resets in 3 hr' },
  { scenario: 'within 7 days', at: new Date(2026, 9, 17, 16, 0).toISOString(), wording: 'Resets Sat 4:00 PM' },
  { scenario: 'later this year', at: new Date(2026, 9, 25, 16, 0).toISOString(), wording: 'Resets Oct 25 at 4:00 PM' },
  { scenario: 'another year', at: new Date(2027, 0, 15, 9, 5).toISOString(), wording: 'Resets Jan 15, 2027 at 9:05 AM' },
])('a reset $scenario reads "$wording"', ({ at, wording }) => {
  expect(formatReset(at, now)).toBe(wording)
})
