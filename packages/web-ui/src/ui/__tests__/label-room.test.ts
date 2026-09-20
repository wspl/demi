import { expect, test } from 'bun:test'
import { compactLabels } from '../label-room'

const host = { width: 80, order: 0 }
const directory = { width: 60, order: 1 }

test('labels show while the protected element keeps its least width', () => {
  expect(compactLabels(10, [{ ...host, compact: false }, { ...directory, compact: false }]))
    .toEqual([false, false])
})

test('the later label gives way first, and only as many as needed', () => {
  // 30px short with both shown: hiding the directory's 60px is enough.
  expect(compactLabels(-30, [{ ...host, compact: false }, { ...directory, compact: false }]))
    .toEqual([false, true])
  // 100px short: both go.
  expect(compactLabels(-100, [{ ...host, compact: false }, { ...directory, compact: false }]))
    .toEqual([true, true])
})

test('a hidden label returns only when the room holds it, so the answer is stable', () => {
  // Both hidden with 90px to spare: the host returns, the directory does not fit after it.
  const once = compactLabels(90, [{ ...host, compact: true }, { ...directory, compact: true }])
  expect(once).toEqual([false, true])
  // Laid out that way the room is 90 - 80 = 10; asking again changes nothing.
  expect(compactLabels(10, [{ ...host, compact: false }, { ...directory, compact: true }]))
    .toEqual(once)
})

test('a later label never shows while an earlier one cannot', () => {
  expect(compactLabels(70, [{ ...host, compact: true }, { ...directory, compact: true }]))
    .toEqual([true, true])
})
