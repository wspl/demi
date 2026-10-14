import { expect, test } from 'bun:test'
import { resumeAtLabel } from '../resume-at-retry'

// The Resume at control of a failure's record (`product.md` § Recovering an
// unfinished turn), against a fixed clock in the local time zone: Sunday,
// October 11, 2026, 8:15 AM.
const now = new Date(2026, 9, 11, 8, 15).getTime()

/** A moment on the local clock. */
function at(...parts: [number, number, number, number, number, number?]): string {
  return new Date(...parts).toISOString()
}

test('a failure of the conversation’s own agent offers to resume at its time, named as the user would say it', () => {
  expect(resumeAtLabel(at(2026, 9, 11, 9, 0), true, true, now)).toBe('Resume at 9:00 AM')
  // Rounded up to the minute, as the record's reset is: never a time before
  // the reset.
  expect(resumeAtLabel(at(2026, 9, 11, 19, 3, 55), true, true, now)).toBe('Resume at 7:04 PM')
  expect(resumeAtLabel(at(2026, 9, 11, 19, 3, 0), true, true, now)).toBe('Resume at 7:03 PM')
  expect(resumeAtLabel(at(2026, 9, 12, 9, 0), true, true, now)).toBe('Resume Mon at 9:00 AM')
  expect(resumeAtLabel(at(2026, 9, 24, 9, 0), true, true, now)).toBe('Resume Oct 24 at 9:00 AM')
})

test('the control is gone once its time has passed, on a subagent’s record, and where the host offers no preference', () => {
  expect(resumeAtLabel(at(2026, 9, 11, 8, 15), true, true, now)).toBeNull()
  expect(resumeAtLabel(at(2026, 9, 11, 7, 0), true, true, now)).toBeNull()
  expect(resumeAtLabel(null, true, true, now)).toBeNull()
  expect(resumeAtLabel(at(2026, 9, 11, 9, 0), false, true, now)).toBeNull()
  expect(resumeAtLabel(at(2026, 9, 11, 9, 0), true, false, now)).toBeNull()
})
