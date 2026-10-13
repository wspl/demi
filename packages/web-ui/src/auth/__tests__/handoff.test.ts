import { expect, test } from 'bun:test'
import { timeLeft } from '../handoff'

test('a code says the minutes and seconds it has left, and nothing once it expired', () => {
  const at = (iso: string) => Date.parse(iso)
  const expiresAt = at('2026-10-13T08:10:00.000Z')
  expect(timeLeft(expiresAt, at('2026-10-13T08:00:00.000Z'))).toBe('10:00')
  expect(timeLeft(expiresAt, at('2026-10-13T08:00:55.200Z'))).toBe('9:05')
  expect(timeLeft(expiresAt, at('2026-10-13T08:09:59.500Z'))).toBe('0:01')
  expect(timeLeft(expiresAt, at('2026-10-13T08:10:00.000Z'))).toBeNull()
})
