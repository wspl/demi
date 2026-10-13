import { expect, test } from 'bun:test'
import { PASSWORD_MIN_LENGTH, generatePassword, isTooShort } from '../password'

// Cost: a few hundred random draws; under a millisecond.
test('a generated password meets the length rule and avoids look-alike characters', () => {
  for (let index = 0; index < 200; index += 1) {
    const password = generatePassword()
    expect(password.length).toBeGreaterThanOrEqual(PASSWORD_MIN_LENGTH)
    expect(isTooShort(password)).toBe(false)
    expect(password).toMatch(/^[a-km-zA-HJ-NP-Z2-9]+$/)
  }
})

test('generated passwords differ', () => {
  expect(new Set(Array.from({ length: 50 }, generatePassword)).size).toBe(50)
})
