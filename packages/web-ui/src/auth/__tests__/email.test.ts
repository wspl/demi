import { expect, test } from 'bun:test'
import { isEmail } from '../email'

test('accepts a trimmed address and refuses an empty or bare name', () => {
  expect(isEmail('zan@example.com')).toBe(true)
  expect(isEmail('  zan@example.com  ')).toBe(true)
  expect(isEmail('Zan')).toBe(false)
  expect(isEmail('')).toBe(false)
  expect(isEmail('zan@example')).toBe(false)
})
