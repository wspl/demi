import { expect, test } from 'bun:test'
import { matcher } from './pattern'

// Cost: pure function; under a millisecond.

test('a path is matched as a part of the text, and /…/flags as an expression', () => {
  expect(matcher('/settings/keyboard')('http://127.0.0.1:3343/settings/keyboard')).toBe(true)
  expect(matcher('/settings/keyboard')('http://127.0.0.1:3343/login')).toBe(false)
  expect(matcher('/LOGIN/i')('http://127.0.0.1:3343/login')).toBe(true)
})
