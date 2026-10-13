import { expect, test } from 'bun:test'
import { accountInitials } from '../account-display'
import { hueToneOf, HUE_TONES } from '../../ui/avatar-tone'

// Cost: pure functions; microseconds.

test('an avatar shows the first letters of the first two words, else the email local part', () => {
  expect(accountInitials('Alex Morgan')).toBe('AM')
  expect(accountInitials('  alex   van  morgan ', 'a@example.com')).toBe('AV')
  expect(accountInitials('Priya')).toBe('P')
  expect(accountInitials('李雷')).toBe('李')
  expect(accountInitials('', "o'neil@example.com")).toBe('O')
  expect(accountInitials('  ', 'bo@example.com')).toBe('B')
  expect(accountInitials('')).toBe('?')
})

test('an account keeps its tone, and ids spread over the eight', () => {
  expect(hueToneOf('u-1')).toBe(hueToneOf('u-1'))
  const used = new Set(Array.from({ length: 200 }, (_, i) => hueToneOf(`user-${i}`)))
  expect(used.size).toBe(HUE_TONES.length)
})
