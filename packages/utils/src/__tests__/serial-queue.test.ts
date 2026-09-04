import { expect, test } from 'bun:test'
import { deferred, SerialQueue } from '../async'

test('serial queues preserve resource order through rejection without blocking another resource', async () => {
  const first = deferred<void>()
  const queue = new SerialQueue()
  const other = new SerialQueue()
  const events: string[] = []
  const a = queue.run(async () => { await first.promise; events.push('first'); throw new Error('failed') })
  a.catch(() => {})
  const b = queue.run(async () => { events.push('second') })
  await other.run(async () => { events.push('other') })
  expect(queue.idle).toBe(false)
  expect(other.idle).toBe(true)
  expect(events).toEqual(['other'])
  first.resolve()
  await expect(a).rejects.toThrow('failed')
  await b
  await queue.settled()
  expect(events).toEqual(['other', 'first', 'second'])
  expect(queue.idle).toBe(true)
})
