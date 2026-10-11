import { expect, test } from 'bun:test'
import { deferred, SerialQueue } from '../async'

test('operations on one thing keep their order through a failure, and other things do not wait for them', async () => {
  const first = deferred<void>()
  const queue = new SerialQueue()
  const events: string[] = []
  const a = queue.run('a', async () => {
    await first.promise
    events.push('a1')
    throw new Error('failed')
  })
  a.catch(() => {})
  const b = queue.run('a', async () => {
    events.push('a2')
  })
  await queue.run('b', async () => {
    events.push('b')
  })
  expect(events).toEqual(['b'])
  first.resolve()
  await expect(a).rejects.toThrow('failed')
  await b
  expect(events).toEqual(['b', 'a1', 'a2'])
})

test('an operation on several things waits for each of them, and holds each', async () => {
  const held = deferred<void>()
  const queue = new SerialQueue()
  const events: string[] = []
  const a = queue.run('a', async () => {
    await held.promise
    events.push('a')
  })
  const both = queue.run(['a', 'b'], async () => {
    events.push('a+b')
  })
  const b = queue.run('b', async () => {
    events.push('b')
  })
  await queue.run('c', async () => {
    events.push('c')
  })
  expect(events).toEqual(['c'])
  held.resolve()
  await Promise.all([a, both, b])
  expect(events).toEqual(['c', 'a', 'a+b', 'b'])
})
