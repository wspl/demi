import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { reportError } from '../errors'
import { ACTION_TOAST_DURATION_MS, dismissToast, holdToasts, releaseToasts, showToast, toasts, TOAST_DURATION_MS } from '../toast'

// Cost: no I/O, fake timers; a few milliseconds.
beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  releaseToasts()
  for (const toast of [...toasts]) dismissToast(toast.id)
  jest.useRealTimers()
})

test('reportError is visible only when userVisible, and it is danger', () => {
  reportError('Failed to send message', new Error('WebSocket is closed'))
  expect(toasts).toEqual([])
  reportError(
    'Failed to send message',
    new Error('WebSocket is closed'),
    { userVisible: true }
  )
  expect(toasts).toHaveLength(1)
  expect(toasts[0]).toMatchObject({
    title: 'Failed to send message',
    message: 'WebSocket is closed',
    tone: 'danger',
  })
})

test('a failure stays until it is closed; a success closes by itself', () => {
  reportError('Could Not Save', new Error('Disk full'), { userVisible: true })
  showToast({ title: 'Copied', tone: 'success' })
  jest.advanceTimersByTime(TOAST_DURATION_MS * 10)
  expect(toasts.map((toast) => toast.title)).toEqual(['Could Not Save'])
})

test('a toast that offers Undo stays at least ten seconds', () => {
  showToast({ title: 'Conversation Archived', tone: 'success', action: { label: 'Undo', run: () => {} } })
  jest.advanceTimersByTime(9999)
  expect(toasts).toHaveLength(1)
  jest.advanceTimersByTime(ACTION_TOAST_DURATION_MS - 9999)
  expect(toasts).toHaveLength(0)
})

test('a toast under the pointer waits, and goes on with the time it had left', () => {
  showToast({ title: 'Copied', tone: 'success' })
  jest.advanceTimersByTime(TOAST_DURATION_MS - 1000)
  holdToasts()
  jest.advanceTimersByTime(TOAST_DURATION_MS * 10)
  expect(toasts).toHaveLength(1)
  releaseToasts()
  jest.advanceTimersByTime(999)
  expect(toasts).toHaveLength(1)
  jest.advanceTimersByTime(1)
  expect(toasts).toHaveLength(0)
})

test('closing the last toast under the pointer lets the next one count down', () => {
  const id = showToast({ title: 'Copied', tone: 'success' })
  holdToasts()
  dismissToast(id)
  showToast({ title: 'Saved', tone: 'success' })
  jest.advanceTimersByTime(TOAST_DURATION_MS)
  expect(toasts).toHaveLength(0)
})
