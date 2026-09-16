import { expect, test } from 'bun:test'
import { formatTimeRemaining } from '../useRelativeTime'

test('countdown labels expire, then count seconds and minutes', () => {
  expect(formatTimeRemaining(-1)).toBe('Expired')
  expect(formatTimeRemaining(0)).toBe('Expired')
  expect(formatTimeRemaining(1)).toBe('0 s left')
  expect(formatTimeRemaining(59_999)).toBe('59 s left')
  expect(formatTimeRemaining(60_000)).toBe('1 min left')
  expect(formatTimeRemaining(58 * 60_000)).toBe('58 min left')
})
