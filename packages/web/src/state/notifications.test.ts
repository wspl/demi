import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/protocol'
import { conversationSummary as summary } from '../__tests__/product-state'
import { answerStart, notificationCauses, type NotificationSettings } from './notifications'

// Cost: pure functions over summaries and blocks; under a millisecond.

const ID = '00000000-0000-4000-8000-000000000001'
const OTHER = '00000000-0000-4000-8000-000000000002'
const ON: NotificationSettings = { enabled: true, turnFinishes: true, turnFails: true, needsPermission: true }
const running = summary(ID, 'Fix the login test', { status: 'running', revision: 4 })

test('a turn that ends with an answer or an error notifies while its conversation is not in front', () => {
  expect(notificationCauses(running, { ...running, status: 'completed' }, ON, null)).toEqual(['turnFinished'])
  expect(notificationCauses(running, { ...running, status: 'error' }, ON, OTHER)).toEqual(['turnFailed'])
  // In front of the user, nothing notifies.
  expect(notificationCauses(running, { ...running, status: 'completed' }, ON, ID)).toEqual([])
})

test('a turn the user stopped, or one interrupted by a restart, notifies nothing', () => {
  expect(notificationCauses(running, { ...running, status: 'stopped' }, ON, null)).toEqual([])
  expect(notificationCauses(running, { ...running, status: 'interrupted' }, ON, null)).toEqual([])
})

test('nothing notifies while notifications are off, or of what the user switched off', () => {
  const ended = { ...running, status: 'completed' as const }
  expect(notificationCauses(running, ended, { ...ON, enabled: false }, null)).toEqual([])
  expect(notificationCauses(running, ended, { ...ON, turnFinishes: false }, null)).toEqual([])
  expect(notificationCauses(running, { ...running, status: 'error' }, { ...ON, turnFails: false }, null)).toEqual([])
})

test('a summary the page sees for the first time tells no change, as after a reload', () => {
  expect(notificationCauses(undefined, { ...running, status: 'completed' }, ON, null)).toEqual([])
})

test('a new or replaced permission request asks for a read of the requests; a decision that leaves none does not', () => {
  const idle = summary(ID, 'Skills', { permissionsRevision: 3 })
  expect(notificationCauses(idle, { ...idle, permissionRequests: 1, permissionsRevision: 4 }, ON, null)).toEqual(['permission'])
  const waiting = { ...idle, permissionRequests: 1, permissionsRevision: 4 }
  expect(notificationCauses(waiting, { ...waiting, permissionsRevision: 5 }, ON, null)).toEqual(['permission'])
  expect(notificationCauses(waiting, { ...waiting, permissionRequests: 0, permissionsRevision: 5 }, ON, null)).toEqual([])
  expect(notificationCauses(idle, { ...waiting }, { ...ON, needsPermission: false }, null)).toEqual([])
})

const createdAt = '2026-10-07T00:00:00.000Z'
const model: ModelSelection = {
  providerId: 'stub',
  model: { id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
  serviceTierId: null,
}
function user(id: string): Block {
  return { type: 'user', id, turnId: id, createdAt, model, content: [{ type: 'text', text: 'Fix it' }], preamble: null }
}
function text(id: string, value: string): Block {
  return { type: 'text', id, createdAt, model, text: value }
}

test('a finished turn says the start of its last answer as plain text, cut to 120 characters', () => {
  const answer = `**Fixed.** The \`login\` test waited for the cookie; it now waits for the redirect. ${'More detail. '.repeat(20)}`
  const start = answerStart([user('u1'), text('a1', 'Earlier answer'), user('u2'), text('a2', 'Looking into it'), text('a3', answer)])
  expect(start.startsWith('Fixed. The login test waited for the cookie; it now waits for the redirect.')).toBe(true)
  expect(start).toHaveLength(120)
  expect(start.endsWith('…')).toBe(true)
  // A turn that ended without text says no earlier turn's answer.
  expect(answerStart([user('u1'), text('a1', 'Earlier answer'), user('u2')])).toBe('')
})
