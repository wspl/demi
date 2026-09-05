import { expect, test } from 'bun:test'
import { shouldDismissMenuTree } from '../menu-context'

test('choice, submenu, and suffix rows keep the menu tree open', () => {
  expect(shouldDismissMenuTree({ isChoice: true, hasSubmenu: false, hasSuffix: false })).toBe(false)
  expect(shouldDismissMenuTree({ isChoice: false, hasSubmenu: true, hasSuffix: false })).toBe(false)
  expect(shouldDismissMenuTree({ isChoice: false, hasSubmenu: false, hasSuffix: true })).toBe(false)
})

test('a plain action row dismisses the menu tree', () => {
  expect(shouldDismissMenuTree({ isChoice: false, hasSubmenu: false, hasSuffix: false })).toBe(true)
})
