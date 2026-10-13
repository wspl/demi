import { expect, test } from 'bun:test'
import { SerialQueue, deferred } from '@demicodes/utils'
import { Slot } from '../slot'

/** A list slot that holds `read` as its first value, as a page that read it. */
function listOf(read: string[], revision?: number): Slot<string[]> {
  const slot = new Slot<string[]>()
  slot.replace(read, revision === undefined ? {} : { revision })
  return slot
}

/** Lets the promises that are ready run. */
async function settled(): Promise<void> {
  for (let turn = 0; turn < 10; turn++) {
    await Promise.resolve()
  }
}

const add = (item: string) => (list: string[]) => [item, ...list]

test('a change shows over the value read at once, in the order it was made, and leaves with its answer applied', () => {
  const slot = listOf(['a'])
  const first = slot.change(add('b'))
  const second = slot.change(add('c'))
  expect(slot.shown.value).toEqual(['c', 'b', 'a'])
  first.send()
  second.send()
  // The answer tells the list as the write left it.
  expect(first.land({ value: (read) => ['b', ...read] })).toBe(true)
  expect(slot.read.value).toEqual(['b', 'a'])
  expect(slot.shown.value).toEqual(['c', 'b', 'a'])
  second.land()
  expect(slot.shown.value).toEqual(['c', 'b', 'a'])
  expect(slot.read.value).toEqual(['c', 'b', 'a'])
})

test('a value read between a write and its answer is not overwritten by the answer; the change stays until the next value read', () => {
  const slot = listOf(['a'])
  const change = slot.change(add('b'))
  change.send()
  // Another page's change reached the slot first, a value that may predate the write.
  slot.replace(['x', 'a'])
  expect(change.land()).toBe(false)
  expect(slot.read.value).toEqual(['x', 'a'])
  expect(slot.shown.value).toEqual(['b', 'x', 'a'])
  // The next value read after the answer holds the write.
  slot.replace(['b', 'x', 'a'])
  expect(slot.shown.value).toEqual(['b', 'x', 'a'])
})

test('a read that went out before the answer landed does not release the change; one that went out after does', () => {
  const slot = listOf(['a'])
  const change = slot.change(add('b'))
  change.send()
  const early = slot.now()
  slot.replace(['a'], { since: early })
  change.land()
  // The early read answers late: it may not hold the write.
  slot.replace(['a'], { since: early })
  expect(slot.shown.value).toEqual(['b', 'a'])
  const late = slot.now()
  slot.replace(['b', 'a'], { since: late })
  expect(slot.shown.value).toEqual(['b', 'a'])
  expect(slot.read.value).toEqual(['b', 'a'])
})

test('a read of the revision already held, requested after the change landed, releases it', () => {
  const slot = listOf(['a'], 3)
  const change = slot.change(add('b'))
  change.send()
  // The summary of the write came first and its read was taken: the list holds the write, at revision 4.
  slot.replace(['b', 'a'], { revision: 4 })
  expect(change.land()).toBe(false)
  expect(slot.shown.value).toEqual(['b', 'b', 'a'])
  // A read requested after the answer holds the same revision: the change is in it.
  slot.replace(['b', 'a'], { revision: 4, since: slot.now() })
  expect(slot.shown.value).toEqual(['b', 'a'])
})

test('an answer that names a revision releases the change with the value of that revision, or at once when it is held', () => {
  const slot = listOf(['a'], 4)
  const change = slot.change(add('b'))
  change.send()
  slot.replace(['x', 'a'], { revision: 5 })
  expect(change.land({ revision: 6 })).toBe(false)
  // A value older than the answer's revision does not release it, nor is an older read taken.
  expect(slot.replace(['older'], { revision: 5 })).toBe(false)
  expect(slot.shown.value).toEqual(['b', 'x', 'a'])
  slot.replace(['b', 'x', 'a'], { revision: 6 })
  expect(slot.shown.value).toEqual(['b', 'x', 'a'])

  const held = slot.change(add('c'))
  held.send()
  expect(held.land({ revision: 6 })).toBe(true)
  expect(slot.shown.value).toEqual(['b', 'x', 'a'])
})

test('a refused change leaves, the slot shows what the backend has, and the refusal is told', async () => {
  const slot = listOf(['a'])
  const queue = new SerialQueue()
  const told: unknown[] = []
  const change = slot.change(add('b'))
  expect(slot.shown.value).toEqual(['b', 'a'])
  const ended = await slot.write(change, (operation) => queue.run('list', operation), async () => {
    throw new Error('refused')
  }, (error) => void told.push(error))
  expect(ended).toBe('refused')
  expect(told).toEqual([new Error('refused')])
  expect(slot.shown.value).toEqual(['a'])
})

test('the writes of one thing go one at a time in order, and those of different things side by side', async () => {
  const slot = listOf([])
  const queue = new SerialQueue()
  const sent: string[] = []
  const answers = new Map<string, ReturnType<typeof deferred>>()
  const write = (item: string, key: string) => slot.write(slot.change(add(item)), (operation) => queue.run(key, operation), () => {
    sent.push(item)
    const answer = deferred()
    answers.set(item, answer)
    return answer.promise
  }, () => {})
  const one = write('one', 'x')
  const two = write('two', 'x')
  const other = write('other', 'y')
  await settled()
  // The second write of x waits for the first; y does not.
  expect(sent).toEqual(['one', 'other'])
  expect(slot.shown.value).toEqual(['other', 'two', 'one'])
  answers.get('one')!.resolve(undefined)
  await one
  await settled()
  expect(sent).toEqual(['one', 'other', 'two'])
  answers.get('two')!.resolve(undefined)
  answers.get('other')!.resolve(undefined)
  expect(await Promise.all([two, other])).toEqual(['landed', 'landed'])
  expect(slot.read.value).toEqual(['other', 'two', 'one'])
})

test('changes of one part wait for that part, not for the reads of another', () => {
  const slot = new Slot<{ a: number; b: number }>()
  slot.replace({ a: 0, b: 0 })
  const change = slot.change((value) => ({ ...value, a: value.a + 1 }), 'a')
  change.send()
  // A value of b says nothing of a's write.
  slot.receive('b', (read) => ({ ...read, b: 7 }))
  expect(change.land()).toBe(true)
  expect(slot.read.value).toEqual({ a: 1, b: 7 })
})
