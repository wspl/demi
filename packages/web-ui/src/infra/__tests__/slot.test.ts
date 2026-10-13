import { expect, test } from 'bun:test'
import { Slot } from '../slot'

// The release rule of every slot the page reads (`web-application.md`
// § Responding to the user). Each test is a few synchronous steps.

/** A list slot that holds `read` as its first value, as a page that read it. */
function listOf(read: string[], revision?: number): Slot<string[]> {
  const slot = new Slot<string[]>()
  slot.replace(read, revision === undefined ? {} : { revisions: { '': revision } })
  return slot
}

const add = (item: string) => (list: string[]) => list.includes(item) ? list : [item, ...list]

test('a change shows over the value read at once, in the order made, and leaves with its answer applied', () => {
  const slot = listOf(['a'])
  const first = slot.change(add('b'))
  const second = slot.change(add('c'))
  expect(slot.shown.value).toEqual(['c', 'b', 'a'])
  first.send()
  second.send()
  expect(first.land({ value: (read) => ['b', ...read] })).toBe(true)
  expect(slot.read.value).toEqual(['b', 'a'])
  expect(slot.shown.value).toEqual(['c', 'b', 'a'])
  expect(second.land()).toBe(true)
  expect(slot.read.value).toEqual(['c', 'b', 'a'])
})

test('an answer that adds one item releases its own change at the revision it names, and no other', () => {
  const slot = listOf([], 1)
  const a = slot.change(add('a'))
  const b = slot.change(add('b'))
  a.send()
  expect(a.land({ revisions: { '': 2 } })).toBe(false)
  b.send()
  expect(b.land({ revisions: { '': 3 } })).toBe(false)
  // A read asked for before B's answer holds A only: A leaves, B stays.
  slot.replace(['a'], { revisions: { '': 2 } })
  expect(slot.pending()).toEqual([b])
  expect(slot.shown.value).toEqual(['b', 'a'])
  // An older read changes nothing, and never takes B away.
  expect(slot.replace([], { revisions: { '': 1 } })).toBe(false)
  expect(slot.shown.value).toEqual(['b', 'a'])
  slot.replace(['b', 'a'], { revisions: { '': 3 } })
  expect(slot.pending()).toEqual([])
  expect(slot.shown.value).toEqual(['b', 'a'])
})

test('an answer taken as the value releases only its own change; one that waits after a read stays until a read asked for after its answer', () => {
  const slot = listOf(['x'])
  const a = slot.change(add('a'))
  a.send()
  // Another page's change reaches the slot between A's write and its answer.
  slot.replace(['y', 'x'])
  expect(a.land()).toBe(false)
  const b = slot.change(add('b'))
  b.send()
  // B's answer is the list as the backend had it then, which may not hold A.
  expect(b.land({ value: (read) => ['b', ...read] })).toBe(true)
  expect(slot.pending()).toEqual([a])
  expect(slot.shown.value).toEqual(['a', 'b', 'y', 'x'])
  // A read asked for before A landed may not hold it: A stays.
  const early = a.landedAt! - 1
  slot.replace(['b', 'y', 'x'], { since: early })
  expect(slot.shown.value).toEqual(['a', 'b', 'y', 'x'])
  slot.replace(['a', 'b', 'y', 'x'], { since: slot.now() })
  expect(slot.pending()).toEqual([])
})

test('a removal the value read already holds leaves once answered', () => {
  const slot = listOf(['a', 'b'])
  const gone = slot.change((list) => list.includes('a') ? list.filter((item) => item !== 'a') : list)
  gone.send()
  slot.replace(['b'])
  expect(gone.land({ revisions: undefined, value: undefined })).toBe(true)
  expect(slot.shown.value).toEqual(['b'])
})

test('a change of one part waits for that part, not for the reads of another', () => {
  const slot = new Slot<{ a: number; b: number }>()
  slot.replace({ a: 0, b: 0 }, { revisions: { a: 0, b: 0 } })
  const change = slot.change((value) => ({ ...value, a: 1 }), { parts: ['a'] })
  change.send()
  expect(change.land({ revisions: { a: 1, b: 0 } })).toBe(false)
  slot.receive('b', (read) => ({ ...read, b: 7 }), { revision: 1 })
  expect(slot.pending()).toEqual([change])
  slot.receive('a', (read) => ({ ...read, a: 1 }), { revision: 1 })
  expect(slot.pending()).toEqual([])
  expect(slot.shown.value).toEqual({ a: 1, b: 7 })
})

test('a refused change leaves at once and the slot shows what the backend has', () => {
  const slot = listOf(['a'])
  const change = slot.change(add('b'))
  change.send()
  change.drop()
  expect(slot.shown.value).toEqual(['a'])
})

test('a change not sent yet shows each revision of it', () => {
  const slot = listOf([])
  const typing = slot.change(() => ['F'])
  typing.revise(() => ['Fi'])
  expect(slot.shown.value).toEqual(['Fi'])
})
