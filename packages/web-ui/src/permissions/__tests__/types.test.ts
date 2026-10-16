import { expect, test } from 'bun:test'
import {
  afterDecision,
  requestAction,
  requestTitle,
  type PermissionCategoryView,
  type PermissionHostView,
  type PermissionRequestView,
  type RequestView,
} from '../types'

const skills: PermissionCategoryView = { id: 'skills.manage', action: 'manage skills', description: 'Manage Skills.' }
const retired: PermissionCategoryView = { id: 'conversations.read', action: null, description: null }

const organize: PermissionCategoryView = { id: 'conversation.organize', action: 'organize conversations', description: 'Organize.' }
const buildBox: PermissionHostView = { kind: 'device', deviceId: 'd1', name: 'build-box' }
const studio: PermissionHostView = { kind: 'device', deviceId: 'd2', name: 'studio' }

function request(id: string, category: PermissionCategoryView = skills): PermissionRequestView {
  return { kind: 'permission', id, categories: [category], host: null, command: `demi ${id}`, subagent: null }
}

function reaching(id: string, host: PermissionHostView): PermissionRequestView {
  return { kind: 'permission', id, categories: [], host, command: `demi ${id}`, subagent: null }
}

test('the card shows at once what a decision leaves: an allow takes every request it completes, a deny one request', () => {
  const queue = [request('a'), request('b'), request('c', retired)]
  expect(afterDecision(queue, 'b', 'allow').map((each) => each.id)).toEqual(['c'])
  expect(afterDecision(queue, 'a', 'deny').map((each) => each.id)).toEqual(['b', 'c'])
  // Another page decided it first: nothing changes here.
  expect(afterDecision(queue, 'gone', 'allow').map((each) => each.id)).toEqual(['a', 'b', 'c'])
  // A host completes the requests of that host, never one of another host
  // or of a pairing, whose code the page does not know; a category never
  // completes a host's.
  const moves = [
    reaching('attach', buildBox),
    request('rename', organize),
    reaching('again', buildBox),
    reaching('studio', studio),
    reaching('pair', { kind: 'pairing' }),
    request('skills'),
  ]
  expect(afterDecision(moves, 'attach', 'allow').map((each) => each.id)).toEqual(['rename', 'studio', 'pair', 'skills'])
  expect(afterDecision(moves, 'rename', 'allow').map((each) => each.id)).toEqual(['attach', 'again', 'studio', 'pair', 'skills'])
  expect(afterDecision(moves, 'pair', 'allow').map((each) => each.id)).toEqual(['attach', 'rename', 'again', 'studio', 'skills'])
})

test('a credential or an allowance is decided alone, whatever the permission requests around it', () => {
  const queue: RequestView[] = [
    request('a'),
    { kind: 'credential', id: 'c', name: 'Google API key', reason: '', fields: [], keep: '1h', renewal: false },
    { kind: 'allowance', id: 'w', name: 'GitHub token', removed: false },
    request('b'),
  ]
  // An allow of a permission request completes the other permission requests, never the others.
  expect(afterDecision(queue, 'a', 'allow').map((each) => each.id)).toEqual(['c', 'w'])
  expect(afterDecision(queue, 'c', 'allow').map((each) => each.id)).toEqual(['a', 'w', 'b'])
  expect(afterDecision(queue, 'w', 'deny').map((each) => each.id)).toEqual(['a', 'c', 'b'])
})

test('a request of a category asks for its action, and a host’s to reach it', () => {
  expect(requestAction(request('rename', organize))).toBe('organize conversations')
  expect(requestAction(request('gone', retired))).toBe('conversations.read')
  expect(requestTitle(request('rename', organize))).toBe('Allow this conversation to organize conversations?')
  expect(requestTitle(reaching('host', buildBox))).toBe('Allow this conversation to reach build-box?')
  expect(requestTitle(reaching('pair', { kind: 'pairing' }))).toBe('Allow this conversation to pair a computer and reach it?')
})
