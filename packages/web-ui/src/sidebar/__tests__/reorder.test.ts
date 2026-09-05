import { expect, test } from 'bun:test'
import { sidebarDrop } from '../reorder'
import type { SidebarConversation, SidebarProject } from '../types'

const projects: SidebarProject[] = ['a', 'b'].map((id) => ({
  id,
  name: id,
  host: 'mac',
  hostKind: 'device',
  path: `/${id}`,
}))
const conversations: SidebarConversation[] = [
  ['one', 'a', false],
  ['two', 'a', false],
  ['three', 'a', false],
  ['pinned', 'a', true],
  ['other', 'b', false],
].map(([id, projectId, pinned]) => ({
  id: id as string,
  title: id as string,
  projectId: projectId as string,
  pinned: pinned as boolean,
  updatedAt: '',
  status: 'idle',
  unread: false,
}))

test('drop resolves both sides and the end without changing group or pin partition', () => {
  const source = { kind: 'conversation' as const, id: 'one' }
  expect(sidebarDrop(source, { ...source, id: 'two' }, true, projects, conversations)).toEqual({
    ...source,
    beforeId: 'three',
  })
  expect(sidebarDrop(source, { ...source, id: 'three' }, true, projects, conversations)).toEqual({
    ...source,
    beforeId: null,
  })
  expect(sidebarDrop(source, { ...source, id: 'two' }, false, projects, conversations)).toBeNull()
  for (const id of ['one', 'pinned', 'other', 'missing'])
    expect(sidebarDrop(source, { ...source, id }, true, projects, conversations)).toBeNull()
  expect(
    sidebarDrop(source, { kind: 'project', id: 'a' }, true, projects, conversations),
  ).toBeNull()
})

test('project reordering is independent of its conversations', () => {
  expect(
    sidebarDrop(
      { kind: 'project', id: 'b' },
      { kind: 'project', id: 'a' },
      false,
      projects,
      conversations,
    ),
  ).toEqual({ kind: 'project', id: 'b', beforeId: 'a' })
})
