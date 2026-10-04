import { expect, test } from 'bun:test'
import {
  afterDecision,
  categoryTitle,
  type PermissionCategoryView,
  type PermissionRequestView,
} from '../types'

const skills: PermissionCategoryView = { id: 'skills.manage', action: 'manage skills', description: 'Manage skills.' }
const retired: PermissionCategoryView = { id: 'conversations.read', action: null, description: null }

function request(id: string, category = skills): PermissionRequestView {
  return { id, category, command: `demi ${id}`, subagent: null }
}

test('the card shows at once what a decision leaves: an allow takes its whole category, a deny one request', () => {
  const queue = [request('a'), request('b'), request('c', retired)]
  expect(afterDecision(queue, 'b', 'allow').map((each) => each.id)).toEqual(['c'])
  expect(afterDecision(queue, 'a', 'deny').map((each) => each.id)).toEqual(['b', 'c'])
  // Another page decided it first: nothing changes here.
  expect(afterDecision(queue, 'gone', 'allow').map((each) => each.id)).toEqual(['a', 'b', 'c'])
})

test('a category is titled by its action, and by its id once the command set no longer declares it', () => {
  expect(categoryTitle(skills)).toBe('Manage skills')
  expect(categoryTitle(retired)).toBe('conversations.read')
})
