import { expect, test } from 'bun:test'
import { linkedCommitBranch, listsUncommitted } from '../history'

// Cost: pure, under a millisecond.

const branches = { branches: ['feat/cart', 'main', 'old'], current: 'main', head: null }

test('Uncommitted Changes leads the list only while it shows the checked-out branch or a detached HEAD', () => {
  expect(listsUncommitted(null, branches)).toBe(true)
  expect(listsUncommitted('main', branches)).toBe(true)
  expect(listsUncommitted('feat/cart', branches)).toBe(false)
  expect(listsUncommitted(null, { branches: ['main'], current: null, head: '3f2a9c1d' })).toBe(true)
  expect(listsUncommitted('main', { branches: ['main'], current: null, head: '3f2a9c1d' })).toBe(false)
})

test('a linked commit opens on the branch shown, else the checked-out one, else the most recent that has it', () => {
  // Shown, and it has the commit: the list stays.
  expect(linkedCommitBranch('feat/cart', 'main', ['main', 'feat/cart'])).toBe('feat/cart')
  // Shown lacks it: the checked-out branch, which the list follows as null.
  expect(linkedCommitBranch('old', 'main', ['main', 'feat/cart'])).toBe(null)
  // Neither has it: the first of the rest, which are most recently committed first.
  expect(linkedCommitBranch(null, 'main', ['feat/cart', 'old'])).toBe('feat/cart')
  // The checked-out branch picked as shown is followed as null.
  expect(linkedCommitBranch('main', 'main', ['main'])).toBe(null)
  // On no branch: shown alone.
  expect(linkedCommitBranch(null, 'main', [])).toBe(undefined)
})
