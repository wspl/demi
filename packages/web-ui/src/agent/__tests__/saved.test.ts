import { expect, test } from 'bun:test'
import { isMacOS } from '@tiptap/core'
import { asksSaveForLater, savedLabel, savedPreview, saveForLaterKeys } from '../saved'

// What the entry of the messages saved for later reads and which key saves
// the message for later (`conversations.md` § Saved for later). Under 1 ms.

test('the entry counts the saved messages', () => {
  expect([1, 2, 7].map(savedLabel)).toEqual(['1 Saved', '2 Saved', '7 Saved'])
})

test('a row reads the message without its files, which its count stands for', () => {
  const mark = '\uFFFC'
  expect([
    `Compare this screenshot with the one in the docs. ${mark}`,
    `Look at ${mark} and ${mark}${mark}, then fix it`,
    `Compare ${mark}.`,
    `${mark}\n\nSummarize this log`,
  ].map(savedPreview)).toEqual([
    'Compare this screenshot with the one in the docs.',
    'Look at and, then fix it',
    'Compare.',
    'Summarize this log',
  ])
})

test('only the platform’s modifier with S saves, and the tip names that key', () => {
  const mod = isMacOS() ? { metaKey: true } : { ctrlKey: true }
  const other = isMacOS() ? { ctrlKey: true } : { metaKey: true }
  const press = (init: Partial<Parameters<typeof asksSaveForLater>[0]>) =>
    ({ key: 's', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...init })
  expect(asksSaveForLater(press(mod))).toBe(true)
  expect(asksSaveForLater(press({ ...mod, key: 'S' }))).toBe(true)
  expect(asksSaveForLater(press({}))).toBe(false)
  expect(asksSaveForLater(press(other))).toBe(false)
  expect(asksSaveForLater(press({ ...mod, shiftKey: true }))).toBe(false)
  expect(saveForLaterKeys()).toBe(isMacOS() ? '⌘S' : 'Ctrl+S')
})
