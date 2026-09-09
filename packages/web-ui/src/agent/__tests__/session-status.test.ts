import { expect, test } from 'bun:test'
import {
  conversationPageKind,
  sessionPaneStatus,
  sessionShowsReconnectTail,
  sessionStatusCopy,
} from '../session-status'

test('a restore in flight is never an empty conversation', () => {
  expect(sessionPaneStatus('loading', false)).toBe('loading')
  expect(sessionPaneStatus('loading', true)).toBe('loading')
  expect(sessionPaneStatus('ready', false)).toBe('empty')
})

test('reconnecting keeps the transcript and is not a pane or an empty conversation', () => {
  expect(sessionPaneStatus('reconnecting', true)).toBeNull()
  expect(sessionPaneStatus('reconnecting', false)).toBeNull()
  expect(sessionShowsReconnectTail('reconnecting')).toBe(true)
  expect(sessionShowsReconnectTail('ready')).toBe(false)
})

test('a failed restore is not a missing conversation', () => {
  expect(sessionPaneStatus('failed', false)).toBe('failed')
  expect(sessionStatusCopy('failed').action).toBe('retry')
  expect(conversationPageKind('failed', false)).toBe('failed')
  expect(conversationPageKind('ready', false)).toBe('missing')
})

test('an unknown id waits for the list', () => {
  expect(conversationPageKind('loading', false)).toBe('loading')
  expect(conversationPageKind('ready', true)).toBe('session')
})
