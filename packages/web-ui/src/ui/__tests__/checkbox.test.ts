import { expect, test } from 'bun:test'
import { checkboxMark, nextCheckbox } from '../checkbox'

test('click toggles on and off; it never enters partial', () => {
  expect(nextCheckbox(false)).toEqual({ checked: true, partial: false })
  expect(nextCheckbox(true)).toEqual({ checked: false, partial: false })
  expect(nextCheckbox(true).partial).toBe(false)
})

test('partial is optional and a click leaves it for checked', () => {
  expect(checkboxMark(false, true)).toBe('partial')
  expect(nextCheckbox(false, true)).toEqual({ checked: true, partial: false })
  expect(nextCheckbox(true, true)).toEqual({ checked: true, partial: false })
})
