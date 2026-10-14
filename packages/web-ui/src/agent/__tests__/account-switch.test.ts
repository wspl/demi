import { expect, test } from 'bun:test'
import { accountSwitchText } from '../account-switch'

// The line names both accounts, the window and its reset in the viewer's
// zone, and leaves out what the block does not know (`providers.md`
// § Account rotation). Microseconds.
test('an account switch reads as the accounts, the window and the reset the block knows', () => {
  const now = new Date(2026, 9, 14, 18, 0)
  const resetsAt = new Date(2026, 9, 14, 21, 0).toISOString()
  expect(
    accountSwitchText({ from: 'alex@example.com', to: 'sam@example.com', window: '5h session', resetsAt }, now),
  ).toBe('alex@example.com reached its 5h session limit, resetting at 9:00 PM; continuing with sam@example.com')
  expect(
    accountSwitchText({ from: 'alex@example.com', to: 'sam@example.com', window: null, resetsAt: null }, now),
  ).toBe('alex@example.com reached its usage limit; continuing with sam@example.com')
})
