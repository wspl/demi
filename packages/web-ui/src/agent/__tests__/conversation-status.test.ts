import { expect, test } from 'bun:test'
import { statusTone } from '../conversation-status'

// `product.md` § Conversations and projects, the status dot table. A sidebar row knows whether
// its answer was read; an agent's tab and link do not.
test('a finished conversation is green until its answer is read, then settles; an agent stays green', () => {
  expect(statusTone('done', { read: false })).toBe('success')
  expect(statusTone('done', { read: true })).toBeNull()
  expect(statusTone('done')).toBe('success')
})

test('reading changes only the finished dot', () => {
  for (const read of [false, true]) {
    expect(statusTone('active', { read })).toBe('accent')
    expect(statusTone('error', { read })).toBe('danger')
    expect(statusTone('aborted', { read })).toBeNull()
    expect(statusTone('idle', { read })).toBeNull()
  }
})
