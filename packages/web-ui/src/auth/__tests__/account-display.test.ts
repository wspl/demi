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

/** WCAG relative luminance of a #rrggbb color. */
function luminance(hex: string): number {
  const value = parseInt(hex.slice(1), 16)
  const channel = (shift: number) => {
    const c = ((value >> shift) & 255) / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  }
  return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

test('white letters keep 4.5:1 where a 24 px avatar draws them, on every tone in both schemes', () => {
  // The capitals of 11 px letters in 24 px span 31% to 69% down the gradient; the top edge is the lighter one.
  for (const tone of HUE_TONES) {
    for (const [top, bottom] of [tone.light, tone.dark]) {
      const mixed = (shift: number) => {
        const a = (parseInt(top.slice(1), 16) >> shift) & 255
        const b = (parseInt(bottom.slice(1), 16) >> shift) & 255
        return Math.round(a + (b - a) * 0.31)
      }
      const color = `#${[16, 8, 0].map(shift => mixed(shift).toString(16).padStart(2, '0')).join('')}`
      expect(1.05 / (luminance(color) + 0.05)).toBeGreaterThanOrEqual(4.5)
    }
  }
})
