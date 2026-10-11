import { expect, test } from 'bun:test'
import { entriesToTakeOff, layersKeptByBack } from '../back-close'

test('Back closes the layers above the entry it lands on, and every layer when it lands on none of theirs', () => {
  expect(layersKeptByBack(['sheet', 'submenu'], 'sheet')).toBe(1)
  expect(layersKeptByBack(['panel', 'sheet', 'submenu'], 'panel')).toBe(1)
  expect(layersKeptByBack(['sheet'], null)).toBe(0)
  expect(layersKeptByBack(['sheet'], 'gone')).toBe(0)
})

test('layers closed another way take off their entries only from the top, where they are', () => {
  expect(entriesToTakeOff([{ open: true }, { open: false }, { open: false }])).toBe(2)
  // A layer under an open one keeps its entry until the one above closes too.
  expect(entriesToTakeOff([{ open: false }, { open: true }])).toBe(0)
  expect(entriesToTakeOff([])).toBe(0)
})
