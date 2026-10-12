import { expect, test } from 'bun:test'
import { statusTone } from '../conversation-status'

// `product.md` § Conversations and projects, the status dot table. A sidebar row knows whether
// its answer or failure was read; an agent's tab and link do not.
test('a finished or failed conversation is green or red until it is read, then settles; an agent keeps its tone', () => {
  expect(statusTone('done', { read: false })).toBe('success')
  expect(statusTone('done', { read: true })).toBeNull()
  expect(statusTone('error', { read: false })).toBe('danger')
  expect(statusTone('error', { read: true })).toBeNull()
  expect(statusTone('done')).toBe('success')
  expect(statusTone('error')).toBe('danger')
})

test('reading changes neither the running dot nor the settled ones', () => {
  for (const read of [false, true]) {
    expect(statusTone('active', { read })).toBe('accent')
    expect(statusTone('aborted', { read })).toBeNull()
    expect(statusTone('idle', { read })).toBeNull()
  }
})
