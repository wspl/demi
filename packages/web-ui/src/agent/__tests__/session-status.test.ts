import { expect, test } from 'bun:test'
import {
  conversationPageKind,
  sessionFailureNotice,
  resumeWaitsFor,
  turnRecovery,
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

test('the dock offers Resume after an error, Continue after a Stop, and nothing while running or finished', () => {
  const answered = [{ type: 'user' }, { type: 'text' }]
  expect(turnRecovery('idle', [...answered, { type: 'error' }])).toBe('resume')
  expect(turnRecovery('idle', [...answered, { type: 'abort' }])).toBe('continue')
  expect(turnRecovery('idle', answered)).toBeNull()
  expect(turnRecovery('idle', [])).toBeNull()
  expect(turnRecovery('running', [...answered, { type: 'error' }])).toBeNull()
})

// A Compact the user asked for whose summary request failed: its error record
// ended no turn, so the dock says what the turn before it needs.
test('a failure that ended no turn offers no Resume and keeps the recovery of the turn before it', () => {
  const failedCompact = { type: 'error', outsideTurn: true }
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'text' }, failedCompact])).toBeNull()
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'error' }, failedCompact])).toBe('resume')
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'abort' }, failedCompact])).toBe('continue')
})

// The user compacted after a turn ended: the divider is the last block, and
// the turn behind it is as it was.
test('a compaction keeps the recovery of the turn before it and offers none of its own', () => {
  const compacted = [{ type: 'compaction_boundary' }, { type: 'compaction_marker' }]
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'error' }, ...compacted])).toBe('resume')
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'abort' }, ...compacted])).toBe('continue')
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'text' }, ...compacted])).toBeNull()
})

// A turn left undone because its Host went offline: Resume continues it
// wherever the conversation runs now, so it waits only for that Host.
test('Resume after a turn its offline Host left unfinished waits for the current primary Host', () => {
  const offline = {
    type: 'error',
    code: 'host_offline',
    device: { id: 'mac', name: 'MacBook Pro' },
  }
  const mac = { id: 'mac', name: 'MacBook Pro', kind: 'device' as const }
  const blocks = [{ type: 'user' }, { type: 'text' }, offline]
  expect(turnRecovery('idle', blocks)).toBe('resume')
  expect(resumeWaitsFor(blocks, { ...mac, state: 'offline' })).toBe('MacBook Pro')
  expect(resumeWaitsFor(blocks, { ...mac, state: 'online' })).toBeNull()
  // Moved to another Host while the device is still away: Resume goes there.
  expect(resumeWaitsFor(blocks, { id: 'studio', name: 'Studio PC', kind: 'device', state: 'online' })).toBeNull()
  // Moved to a device that has since gone offline: Resume waits for it and names it.
  expect(resumeWaitsFor(blocks, { id: 'studio', name: 'Studio PC', kind: 'device', state: 'offline' })).toBe('Studio PC')
  // A stopped Cloud wakes for the resume.
  expect(resumeWaitsFor(blocks, { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'offline' })).toBeNull()
  // Another failure's Resume waits for nothing.
  expect(resumeWaitsFor([{ type: 'user' }, { type: 'error', code: 'overloaded' }], { ...mac, state: 'offline' })).toBeNull()
  // Behind a compaction the record still holds Resume back.
  expect(resumeWaitsFor([...blocks, { type: 'compaction_boundary' }], { ...mac, state: 'offline' })).toBe('MacBook Pro')
})
