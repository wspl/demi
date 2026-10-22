import { expect, test } from 'bun:test'
import { backendUnreachable } from '../connection'

// Whether the page reaches the backend; the product state's tests play the
// channel that leads to each. About 1 ms.

const reachable = { online: true, restarting: false, failedAttempts: 0, hasSnapshot: true }

test('no network is unreachable, whatever the channel did', () => {
  expect(backendUnreachable({ ...reachable, online: false })).toBe(true)
  expect(backendUnreachable({ ...reachable, online: false, restarting: true })).toBe(true)
  expect(backendUnreachable({ ...reachable, online: false, failedAttempts: 3 })).toBe(true)
  expect(backendUnreachable({ ...reachable, online: false, hasSnapshot: false })).toBe(true)
})

test('a page that reaches the backend is not unreachable', () => {
  expect(backendUnreachable(reachable)).toBe(false)
})
