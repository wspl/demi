import { expect, test } from 'bun:test'
import { cssColorToRgba } from '../useTerminalTheme'

test('css rgb and space-separated rgb both take an alpha', () => {
  expect(cssColorToRgba('rgb(96, 165, 250)', 0.34)).toBe('rgba(96, 165, 250, 0.34)')
  expect(cssColorToRgba('rgb(96 165 250)', 0.34)).toBe('rgba(96, 165, 250, 0.34)')
})
