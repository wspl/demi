import { expect, test } from 'bun:test'
import { reportError } from '../errors'
import { dismissToast, showToast, toasts } from '../toast'

function resetToasts() {
  for (const toast of [...toasts]) dismissToast(toast.id)
}

test('showToast appends a notice and dismissToast removes it', () => {
  resetToasts()
  const id = showToast({ title: 'Copied', durationMs: 0 })
  expect(toasts).toHaveLength(1)
  expect(toasts[0]).toMatchObject({ id, title: 'Copied', tone: 'neutral' })
  dismissToast(id)
  expect(toasts).toEqual([])
})

test('reportError is visible only when userVisible, and it is danger', () => {
  resetToasts()
  reportError('Failed to send message', new Error('WebSocket is closed'))
  expect(toasts).toEqual([])
  reportError('Failed to send message', new Error('WebSocket is closed'), { userVisible: true })
  expect(toasts).toHaveLength(1)
  expect(toasts[0]).toMatchObject({
    title: 'Failed to send message',
    message: 'WebSocket is closed',
    tone: 'danger',
  })
  resetToasts()
})
