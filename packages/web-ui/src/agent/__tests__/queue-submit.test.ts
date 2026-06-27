import { expect, test } from 'bun:test'
import { queuedMessageIdForEmptySubmit } from '../queue-submit'

test('empty submit with no queue has no queued message target', () => {
  expect(queuedMessageIdForEmptySubmit([])).toBeNull()
})

test('empty submit targets the last queued message', () => {
  expect(queuedMessageIdForEmptySubmit([
    { id: 'queued-first' },
    { id: 'queued-second' },
    { id: 'queued-last' },
  ])).toBe('queued-last')
})
