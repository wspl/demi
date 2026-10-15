import { expect, test } from 'bun:test'
import { createPageI18n } from '../../i18n/text'
import { accountSwitchText } from '../account-switch'

// The line names both accounts, the window in the viewer's language and
// its reset in the viewer's zone, and leaves out what the block does not
// know (`providers.md` § Account rotation). Microseconds.
test('an account switch reads as the accounts, the window and the reset the block knows', () => {
  const now = new Date(2026, 9, 14, 18, 0)
  const resetsAt = new Date(2026, 9, 14, 21, 0).toISOString()
  const weeklyOpus = { kind: 'limit', periodSeconds: 7 * 86_400, scope: { kind: 'model', label: 'Opus' } } as const
  const fiveHours = { kind: 'limit', periodSeconds: 5 * 3600, scope: null } as const
  expect(
    accountSwitchText({ from: 'alex@example.com', to: 'sam@example.com', window: weeklyOpus, resetsAt }, now),
  ).toBe('alex@example.com reached its weekly Opus limit, resetting at 9:00 PM. Continuing with sam@example.com.')
  expect(
    accountSwitchText({ from: 'alex@example.com', to: 'sam@example.com', window: null, resetsAt: null }, now),
  ).toBe('alex@example.com reached its usage limit. Continuing with sam@example.com.')
  createPageI18n('zh-Hans')
  expect(
    accountSwitchText({ from: 'alex@example.com', to: 'sam@example.com', window: fiveHours, resetsAt: null }, now),
  ).toBe('alex@example.com 已达到 5 小时限额，改用 sam@example.com 继续。')
  createPageI18n('en')
})
