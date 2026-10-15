import { expect, test } from 'bun:test'
import {
  continueWithOffer,
  conversationPageKind,
  sessionFailureNotice,
  resumeWaitsFor,
  turnRecovery,
  endsWithErrorRecord,
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
  expect(sessionFailureNotice('failed', { message: 'Socket closed', requestId: null }, true, false)).toEqual({
    label: 'Socket closed',
    retry: true,
    requestId: null,
  })
})

test('the dock notice never repeats a pane or an error record', () => {
  const closed = { message: 'Socket closed', requestId: null }
  expect(sessionFailureNotice('failed', closed, false, false)).toBeNull()
  expect(sessionFailureNotice('loading', closed, true, false)).toBeNull()
  expect(sessionFailureNotice('ready', { message: 'Overloaded', requestId: null }, true, true)).toBeNull()
  // A refused frame names the socket's request, which the notice shows.
  expect(sessionFailureNotice('ready', { message: 'Request refused', requestId: 'req_k3v9x2m7qd4h8w1p' }, true, false)).toEqual({
    label: 'Request refused',
    retry: false,
    requestId: 'req_k3v9x2m7qd4h8w1p',
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

// A root's failed turn holds what arrives afterwards and writes it behind its
// record: the turn still offers Resume, which answers it. A reports block that
// opens a turn of its own is that turn's start, not late input.
test('agent messages and reports written behind a failure keep its Resume', () => {
  const late = [{ type: 'agent_message' }, { type: 'agent_message' }, { type: 'wakeup', placement: 'steer' }]
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'error' }, ...late])).toBe('resume')
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'text' }, ...late])).toBeNull()
  expect(turnRecovery('idle', [{ type: 'user' }, { type: 'error' }, { type: 'wakeup', placement: 'new_turn' }])).toBeNull()
})

// The failure's `error` frame names it while the page is live, and the
// record says it too: receipts written behind the record do not make the
// tail notice repeat it, as a reload, which has no frame, never did.
test('a failure the record says is not repeated under the receipts written behind it', () => {
  const failed = { message: 'You’ve hit your session limit', requestId: 'req_1' }
  const late = [{ type: 'agent_message' }, { type: 'agent_message' }, { type: 'wakeup', placement: 'steer' }]
  const behind = endsWithErrorRecord([{ type: 'user' }, { type: 'error' }, ...late])
  expect(behind).toBe(true)
  expect(sessionFailureNotice('ready', failed, true, behind)).toBeNull()
  // A new turn that the reports opened is not the record's.
  expect(endsWithErrorRecord([{ type: 'user' }, { type: 'error' }, { type: 'wakeup', placement: 'new_turn' }])).toBe(false)
  expect(endsWithErrorRecord([{ type: 'user' }, { type: 'text' }, ...late])).toBe(false)
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
  const mac = { id: 'mac', name: 'MacBook Pro', kind: 'device' as const, mark: null }
  const blocks = [{ type: 'user' }, { type: 'text' }, offline]
  expect(turnRecovery('idle', blocks)).toBe('resume')
  expect(resumeWaitsFor(blocks, { ...mac, state: 'offline' })).toBe('MacBook Pro')
  expect(resumeWaitsFor(blocks, { ...mac, state: 'online' })).toBeNull()
  // Moved to another Host while the device is still away: Resume goes there.
  expect(resumeWaitsFor(blocks, { id: 'studio', name: 'Studio PC', kind: 'device', state: 'online', mark: null })).toBeNull()
  // Moved to a device that has since gone offline: Resume waits for it and names it.
  expect(resumeWaitsFor(blocks, { id: 'studio', name: 'Studio PC', kind: 'device', state: 'offline', mark: null })).toBe('Studio PC')
  // A stopped Cloud wakes for the resume.
  expect(resumeWaitsFor(blocks, { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'offline', mark: null })).toBeNull()
  // Another failure's Resume waits for nothing.
  expect(resumeWaitsFor([{ type: 'user' }, { type: 'error', code: 'overloaded' }], { ...mac, state: 'offline' })).toBeNull()
  // Behind a compaction the record still holds Resume back.
  expect(resumeWaitsFor([...blocks, { type: 'compaction_boundary' }], { ...mac, state: 'offline' })).toBe('MacBook Pro')
})

// Continue with reads the unfinished turn's record as Resume does: a
// child's report or a compaction written behind the usage limit keeps it,
// and another end or a running turn offers none (`providers.md` § Account
// rotation). Microseconds.
test('a usage limit offers to continue with another account while late input follows it', () => {
  const model = { providerId: 'claude' }
  const failures = { e1: { continueWith: { credentialId: 'a2', label: 'sam@example.com' } } }
  const offer = { providerId: 'claude', credentialId: 'a2', label: 'sam@example.com' }
  const failed = [{ type: 'user' }, { type: 'error', id: 'e1', code: 'usage_limit', model }]
  expect(continueWithOffer('idle', failed, failures)).toEqual(offer)
  const late = [
    ...failed,
    { type: 'agent_message' },
    { type: 'wakeup', placement: 'steer' },
    { type: 'compaction_marker' },
  ]
  expect(continueWithOffer('idle', late, failures)).toEqual(offer)
  expect(continueWithOffer('running', late, failures)).toBeNull()
  expect(continueWithOffer('idle', [...failed, { type: 'user' }, { type: 'text' }], failures)).toBeNull()
  expect(continueWithOffer('idle', [{ type: 'user' }, { type: 'error', id: 'e1', code: 'rate_limited', model }], failures)).toBeNull()
  expect(continueWithOffer('idle', failed, {})).toBeNull()
})
