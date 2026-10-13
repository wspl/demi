import { expect, test } from 'bun:test'
import { isMacOS } from '@tiptap/core'
import { asksSaveDraft, draftsLabel, saveDraftKeys } from '../drafts'

// What the drafts' entry reads and which key saves the message as a draft
// (`product.md` § Drafts). Under 1 ms.

test('the entry counts the drafts, and reads "1 Draft" for one', () => {
  expect([1, 2, 7].map(draftsLabel)).toEqual(['1 Draft', '2 Drafts', '7 Drafts'])
})

test('only the platform’s modifier with S saves, and the tip names that key', () => {
  const mod = isMacOS() ? { metaKey: true } : { ctrlKey: true }
  const other = isMacOS() ? { ctrlKey: true } : { metaKey: true }
  const press = (init: Partial<Parameters<typeof asksSaveDraft>[0]>) =>
    ({ key: 's', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...init })
  expect(asksSaveDraft(press(mod))).toBe(true)
  expect(asksSaveDraft(press({ ...mod, key: 'S' }))).toBe(true)
  expect(asksSaveDraft(press({}))).toBe(false)
  expect(asksSaveDraft(press(other))).toBe(false)
  expect(asksSaveDraft(press({ ...mod, shiftKey: true }))).toBe(false)
  expect(saveDraftKeys()).toBe(isMacOS() ? '⌘S' : 'Ctrl+S')
})
