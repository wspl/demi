import { expect, test } from 'bun:test'
import { menuLeadingSlot } from '../menu-context'

// Where a checked row's check goes is the menu's to decide from its rows, as
// macOS puts it (`MenuLayout`): the Workflow menu's rows have no icons, the
// Run On menu's have. A few microseconds.

test('a menu whose rows have no icons puts the check in the leading slot', () => {
  const workflow = [{ hasIcon: false, checkable: true }, { hasIcon: false, checkable: true }]
  expect(menuLeadingSlot(workflow)).toEqual({ iconless: true, checkLeading: true })
})

test('a menu whose rows have icons keeps the icon there and the check at the end', () => {
  const runOn = [{ hasIcon: true, checkable: true }, { hasIcon: true, checkable: true }]
  expect(menuLeadingSlot(runOn)).toEqual({ iconless: false, checkLeading: false })
})

test('one row with an icon or status dot is enough for the icon column', () => {
  const mixed = [{ hasIcon: false, checkable: true }, { hasIcon: true, checkable: true }]
  expect(menuLeadingSlot(mixed)).toEqual({ iconless: false, checkLeading: false })
})

test('rows that cannot be checked reserve no check slot', () => {
  const actions = [{ hasIcon: false, checkable: false }, { hasIcon: false, checkable: false }]
  expect(menuLeadingSlot(actions)).toEqual({ iconless: true, checkLeading: false })
})

test('a menu forced iconless still follows the rows for its checks', () => {
  expect(menuLeadingSlot([{ hasIcon: true, checkable: true }], true)).toEqual({ iconless: true, checkLeading: true })
})
