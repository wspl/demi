import { expect, test } from 'bun:test'
import { cssColorToRgba, terminalPalette } from '../useTerminalTheme'

test('selection is a transparent accent, not the ground', () => {
  const dark = terminalPalette('dark')
  const light = terminalPalette('light')
  expect(dark.selectionBackground).toBe('rgba(96, 165, 250, 0.34)')
  expect(dark.selectionInactiveBackground).toBe(dark.selectionBackground)
  expect(light.selectionBackground).toBe('rgba(37, 99, 235, 0.34)')
  expect(light.selectionBackground).not.toBe(light.background)
})

test('light brights stay darker than the paper so they remain readable', () => {
  const light = terminalPalette('light')
  expect(light.brightGreen).toBe('#15803d')
  expect(light.brightWhite).toBe(light.black)
  expect(light.green).not.toBe(light.background)
})

test('css rgb and space-separated rgb both take an alpha', () => {
  expect(cssColorToRgba('rgb(96, 165, 250)', 0.34)).toBe('rgba(96, 165, 250, 0.34)')
  expect(cssColorToRgba('rgb(96 165 250)', 0.34)).toBe('rgba(96, 165, 250, 0.34)')
  expect(cssColorToRgba('oklch(76% 0.15 255)', 0.34)).toBe('oklch(76% 0.15 255)')
})
