import { expect, test } from 'bun:test'
import { disabledTooltip } from '../disabled'

test('a disabled control shows its trimmed reason', () => {
  expect(disabledTooltip(true, 'In development')).toBe('In development')
  expect(disabledTooltip(true, '  In development  ')).toBe('In development')
})

test('an enabled control or an empty reason has no tooltip', () => {
  expect(disabledTooltip(false, 'In development')).toBeUndefined()
  expect(disabledTooltip(undefined, 'In development')).toBeUndefined()
  expect(disabledTooltip(true, '')).toBeUndefined()
  expect(disabledTooltip(true, '   ')).toBeUndefined()
  expect(disabledTooltip(true, undefined)).toBeUndefined()
})
