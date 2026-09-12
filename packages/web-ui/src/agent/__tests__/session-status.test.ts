import { expect, test } from 'bun:test'
import {
  conversationPageKind,
  sessionFailureNotice,
  sessionPaneStatus,
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
})

test('a failed reconnect keeps the transcript and names the failure in the dock', () => {
  expect(sessionPaneStatus('failed', true)).toBeNull()
  expect(sessionFailureNotice('failed', 'Socket closed', true, false)).toEqual({
    label: 'Socket closed',
    retry: true,
  })
})

test('the dock notice never repeats a pane or an error record', () => {
  expect(sessionFailureNotice('failed', 'Socket closed', false, false)).toBeNull()
  expect(sessionFailureNotice('loading', 'Socket closed', true, false)).toBeNull()
  expect(sessionFailureNotice('ready', 'Overloaded', true, true)).toBeNull()
  expect(sessionFailureNotice('ready', 'Request refused', true, false)).toEqual({
    label: 'Request refused',
    retry: false,
  })
  expect(sessionFailureNotice('ready', null, true, false)).toBeNull()
})

test('a failed restore is not a missing conversation', () => {
  expect(sessionPaneStatus('failed', false)).toBe('failed')
  expect(sessionStatusCopy('failed').action).toBe('retry')
  expect(conversationPageKind('failed', false)).toBe('failed')
  expect(conversationPageKind('ready', false)).toBe('missing')
})

test('only a pane with nothing under it offers to start a conversation', () => {
  expect(sessionStatusCopy('empty').action).toBeUndefined()
  expect(sessionStatusCopy('none').action).toBe('create')
  expect(sessionStatusCopy('missing').action).toBe('create')
})

test('an unknown id waits for the list', () => {
  expect(conversationPageKind('loading', false)).toBe('loading')
  expect(conversationPageKind('ready', true)).toBe('session')
  expect(conversationPageKind('loading', true)).toBe('session')
  expect(conversationPageKind('failed', true)).toBe('session')
})
