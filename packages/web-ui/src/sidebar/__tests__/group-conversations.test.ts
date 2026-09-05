import { expect, test } from 'bun:test'
import { projectGroups } from '../group-conversations'
import type { SidebarConversation, SidebarProject } from '../types'

test('conversation activity and pinning never reorder projects, including empty projects', () => {
  const projects: SidebarProject[] = ['first', 'second', 'empty'].map((id) => ({
    id,
    name: id,
    host: 'device',
    hostKind: 'device',
    path: `/${id}`,
  }))
  const conversations: SidebarConversation[] = [
    {
      id: 'older',
      title: 'Older',
      projectId: 'first',
      updatedAt: '2026-01-01',
      status: 'done',
      pinned: false,
      unread: false,
    },
    {
      id: 'new',
      title: 'New',
      projectId: 'second',
      updatedAt: '2026-09-06',
      status: 'done',
      pinned: true,
      unread: false,
    },
  ]
  expect(projectGroups(projects, conversations).map((group) => group.project.id)).toEqual([
    'first',
    'second',
    'empty',
  ])
  conversations[0]!.updatedAt = '2027-01-01'
  expect(projectGroups(projects, conversations).map((group) => group.project.id)).toEqual([
    'first',
    'second',
    'empty',
  ])
  expect(projectGroups(projects, [])[2]!.items).toEqual([])
})
