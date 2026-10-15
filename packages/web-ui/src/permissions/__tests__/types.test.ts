import { expect, test } from 'bun:test'
import {
  afterDecision,
  categoryTitle,
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

function request(id: string, ...categories: PermissionCategoryView[]): PermissionRequestView {
  return { kind: 'permission', id, categories: categories.length ? categories : [skills], host: null, command: `demi ${id}`, subagent: null }
}

function reaching(id: string, host: PermissionHostView, ...categories: PermissionCategoryView[]): PermissionRequestView {
  return { kind: 'permission', id, categories, host, command: `demi ${id}`, subagent: null }
}

test('the card shows at once what a decision leaves: an allow takes every request it completes, a deny one request', () => {
  const queue = [request('a'), request('b'), request('c', retired)]
  expect(afterDecision(queue, 'b', 'allow').map((each) => each.id)).toEqual(['c'])
  expect(afterDecision(queue, 'a', 'deny').map((each) => each.id)).toEqual(['b', 'c'])
  // Another page decided it first: nothing changes here.
  expect(afterDecision(queue, 'gone', 'allow').map((each) => each.id)).toEqual(['a', 'b', 'c'])
  // Allowing a category and a host completes a request of either; one of
  // the category alone completes no request that also needs the host, and
  // a host completes no request of another host or of a pairing, whose code
  // the page does not know.
  const moves = [
    reaching('move', buildBox, organize),
    request('rename', organize),
    reaching('attach', buildBox),
    reaching('studio', studio),
    reaching('pair', { kind: 'pairing' }),
    request('skills'),
  ]
  expect(afterDecision(moves, 'move', 'allow').map((each) => each.id)).toEqual(['studio', 'pair', 'skills'])
  expect(afterDecision(moves, 'rename', 'allow').map((each) => each.id)).toEqual(['move', 'attach', 'studio', 'pair', 'skills'])
  expect(afterDecision(moves, 'pair', 'allow').map((each) => each.id)).toEqual(['move', 'rename', 'attach', 'studio', 'skills'])
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

test('a request of several parts asks for their actions joined with "and"; a host alone asks to reach it', () => {
  expect(requestAction(request('two', organize, skills))).toBe('organize conversations and manage skills')
  expect(requestAction(request('gone', retired))).toBe('conversations.read')
  expect(requestTitle(reaching('move', buildBox, organize))).toBe('Allow this conversation to organize conversations and reach build-box?')
  expect(requestTitle(reaching('host', buildBox))).toBe('Allow this conversation to reach build-box?')
  expect(requestTitle(reaching('pair', { kind: 'pairing' }))).toBe('Allow this conversation to pair a computer and reach it?')
})

test('a category is titled by its action, and by its id once the command set no longer declares it', () => {
  expect(categoryTitle(skills)).toBe('Manage Skills')
  expect(categoryTitle(organize)).toBe('Organize Conversations')
  expect(categoryTitle(retired)).toBe('conversations.read')
})
