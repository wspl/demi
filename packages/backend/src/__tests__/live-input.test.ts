import { expect, test } from 'bun:test'
import { LiveInput } from '../runner/live-input'

test('live stdin requests one chunk for each read and rejects unsolicited bytes', async () => {
  let pulls = 0
  const input = new LiveInput(() => { pulls += 1 })
  expect(() => input.push(new Uint8Array([1]))).toThrow('Unrequested')
  const iterator = input.stream()[Symbol.asyncIterator]()
  const first = iterator.next()
  expect(pulls).toBe(1)
  await expect(iterator.next()).rejects.toThrow('Concurrent')
  input.push(new Uint8Array([0, 255]))
  expect(await first).toEqual({ done: false, value: new Uint8Array([0, 255]) })
  const second = iterator.next()
  expect(pulls).toBe(2)
  input.close()
  expect((await second).done).toBe(true)
  expect((await iterator.next()).done).toBe(true)
  expect(pulls).toBe(2)
})
