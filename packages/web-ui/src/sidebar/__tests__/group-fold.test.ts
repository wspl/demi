import { expect, test } from 'bun:test'
import { projectGroups, plainConversations } from '../group-conversations'
import { labelWaiting, listLines, type SidebarGroup } from '../list-model'
import type { SidebarConversation, SidebarProject } from '../types'

// Cost: pure functions, no DOM; milliseconds.

const projects: SidebarProject[] = [
  { id: 'p-demi', name: 'demi', host: 'Cloud', path: '/demi', hostKind: 'cloud' },
  { id: 'p-notes', name: 'notes', host: 'Cloud', path: '/notes', hostKind: 'cloud' },
]

function row(id: string, projectId: string | null, needsYou = false): SidebarConversation {
  return { id, title: id, updatedAt: '2026-10-10T09:00:00Z', status: 'done', projectId, pinned: false, unread: false, needsYou }
}

const conversations = [
  row('plain-a', null),
  row('plain-b', null),
  row('demi-a', 'p-demi'),
  row('demi-b', 'p-demi'),
  row('notes-a', 'p-notes'),
]

/** The ids a reader sees, in order: the lines no fold hides. */
function shown(foldedProjects: string[], foldedGroups: SidebarGroup[], openId: string | null): string[] {
  return listLines(
    plainConversations(conversations),
    projectGroups(projects, conversations),
    new Set(foldedProjects),
    new Set(foldedGroups),
    openId,
  ).filter((line) => !line.hidden).map((line) => line.id)
}

test('a folded group shows its label alone but keeps the conversation open in the page', () => {
  const cases: { name: string; projects: string[]; groups: SidebarGroup[]; open: string | null; shown: string[] }[] = [
    { name: 'both open', projects: [], groups: [], open: 'plain-b', shown: ['plain-a', 'plain-b', 'p-demi', 'demi-a', 'demi-b', 'p-notes', 'notes-a'] },
    { name: 'conversations folded, a plain one open', projects: [], groups: ['conversations'], open: 'plain-b', shown: ['plain-b', 'p-demi', 'demi-a', 'demi-b', 'p-notes', 'notes-a'] },
    { name: 'conversations folded, a project one open', projects: [], groups: ['conversations'], open: 'demi-b', shown: ['p-demi', 'demi-a', 'demi-b', 'p-notes', 'notes-a'] },
    { name: 'projects folded, a project one open', projects: [], groups: ['projects'], open: 'demi-b', shown: ['plain-a', 'plain-b', 'demi-b'] },
    { name: 'projects folded, its project folded too', projects: ['p-demi'], groups: ['projects'], open: 'demi-b', shown: ['plain-a', 'plain-b', 'demi-b'] },
    { name: 'projects folded, a plain one open', projects: [], groups: ['projects'], open: 'plain-a', shown: ['plain-a', 'plain-b'] },
    { name: 'both folded, nothing open', projects: [], groups: ['conversations', 'projects'], open: null, shown: [] },
    { name: 'a folded project in an open group still hides the open row', projects: ['p-demi'], groups: [], open: 'demi-b', shown: ['plain-a', 'plain-b', 'p-demi', 'p-notes', 'notes-a'] },
  ]
  for (const each of cases) {
    expect({ name: each.name, shown: shown(each.projects, each.groups, each.open) }).toEqual({ name: each.name, shown: each.shown })
  }
})

test('a folded group’s label carries the dot of a conversation in it that waits for the user', () => {
  const waiting = [row('demi-a', 'p-demi'), row('demi-b', 'p-demi', true), row('notes-a', 'p-notes', true)]
  // Folded: the first waiting conversation's dot, whether or not it is open.
  expect(labelWaiting(true, waiting)?.id).toBe('demi-b')
  // Open: the rows show their own dots, so the label carries none.
  expect(labelWaiting(false, waiting)).toBeNull()
  // Folded with nothing waiting: no dot.
  expect(labelWaiting(true, [row('demi-a', 'p-demi')])).toBeNull()
})
