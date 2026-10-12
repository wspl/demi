import { expect, test } from 'bun:test'
import {
  afterDecision,
  categoryTitle,
  requestAction,
  type PermissionCategoryView,
  type PermissionRequestView,
  type RequestView,
} from '../types'

const skills: PermissionCategoryView = { id: 'skills.manage', action: 'manage skills', description: 'Manage Skills.' }
const retired: PermissionCategoryView = { id: 'conversations.read', action: null, description: null }

const organize: PermissionCategoryView = { id: 'conversation.organize', action: 'organize conversations', description: 'Organize.' }
const devices: PermissionCategoryView = { id: 'host.devices', action: 'manage devices', description: 'Devices.' }

function request(id: string, ...categories: PermissionCategoryView[]): PermissionRequestView {
  return { kind: 'permission', id, categories: categories.length ? categories : [skills], command: `demi ${id}`, subagent: null }
}

test('the card shows at once what a decision leaves: an allow takes every request it completes, a deny one request', () => {
  const queue = [request('a'), request('b'), request('c', retired)]
  expect(afterDecision(queue, 'b', 'allow').map((each) => each.id)).toEqual(['c'])
  expect(afterDecision(queue, 'a', 'deny').map((each) => each.id)).toEqual(['b', 'c'])
  // Another page decided it first: nothing changes here.
  expect(afterDecision(queue, 'gone', 'allow').map((each) => each.id)).toEqual(['a', 'b', 'c'])
  // Allowing both categories completes a request of either; one of a single
  // category completes no request that also needs the other.
  const moves = [request('move', organize, devices), request('rename', organize), request('attach', devices), request('skills')]
  expect(afterDecision(moves, 'move', 'allow').map((each) => each.id)).toEqual(['skills'])
  expect(afterDecision(moves, 'rename', 'allow').map((each) => each.id)).toEqual(['move', 'attach', 'skills'])
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

test('a request of several categories asks for their actions joined with "and"', () => {
  expect(requestAction(request('move', organize, devices))).toBe('organize conversations and manage devices')
  expect(requestAction(request('gone', retired))).toBe('conversations.read')
})

test('a category is titled by its action, and by its id once the command set no longer declares it', () => {
  expect(categoryTitle(skills)).toBe('Manage Skills')
  expect(categoryTitle(organize)).toBe('Organize Conversations')
  expect(categoryTitle(retired)).toBe('conversations.read')
})
