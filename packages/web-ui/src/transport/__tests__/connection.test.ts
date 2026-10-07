import { expect, test } from 'bun:test'
import { connectionProblem } from '../connection'

// What the connection banner says (`web-application.md` § A page of another
// build); the product state's tests play the channel that leads to each.
// About 1 ms.

const reachable = { online: true, restarting: false, failedAttempts: 0, hasSnapshot: true }

test('no network is what the banner says, whatever the channel did', () => {
  expect(connectionProblem({ ...reachable, online: false })).toBe('offline')
  expect(connectionProblem({ ...reachable, online: false, restarting: true })).toBe('offline')
  expect(connectionProblem({ ...reachable, online: false, failedAttempts: 3 })).toBe('offline')
  expect(connectionProblem({ ...reachable, online: false, hasSnapshot: false })).toBe('offline')
})

test('a page that reaches the backend shows no banner', () => {
  expect(connectionProblem(reachable)).toBeNull()
})
