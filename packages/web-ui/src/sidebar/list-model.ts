import type { ProjectGroup } from './group-conversations'
import type { SidebarConversation } from './types'

/** One navigable line of the list, in display order: a conversation row or a project header. */
export type ListEntry =
  | {
    kind: 'conversation';
    id: string;
    projectId: string | null
  }
  | {
    kind: 'project';
    id: string
  }

/** The sidebar's two groups, each folding under its label. */
export const SIDEBAR_GROUPS = ['conversations', 'projects'] as const
export type SidebarGroup = (typeof SIDEBAR_GROUPS)[number]

/** A line of the list, and whether its group's fold hides it under the group's label. */
export type ListLine = ListEntry & { hidden: boolean }

/**
 * Every line the list holds, in display order: plain rows, then each project header
 * and its rows.
 *
 * A fold, of a project or of a group, keeps the conversation open in the page in
 * sight, so its place still shows: a folded project lists only that row under its
 * header, and a folded group shows it under the label, in Projects with its
 * project's header above it. A folded group's hidden lines stay listed so their fold
 * can animate. The keyboard reaches the lines that are not hidden.
 */
export function listLines(
  plain: readonly SidebarConversation[],
  groups: readonly ProjectGroup[],
  foldedProjects: ReadonlySet<string>,
  foldedGroups: ReadonlySet<SidebarGroup>,
  openId: string | null,
): ListLine[] {
  const conversationsFolded = foldedGroups.has('conversations')
  const projectsFolded = foldedGroups.has('projects')
  const lines: ListLine[] = plain.map(
    (conversation) => ({
      kind: 'conversation',
      id: conversation.id,
      projectId: null,
      hidden: conversationsFolded && conversation.id !== openId,
    })
  )
  for (const group of groups) {
    const holdsOpen = group.items.some((conversation) => conversation.id === openId)
    lines.push({ kind: 'project', id: group.project.id, hidden: projectsFolded && !holdsOpen })
    const projectFolded = foldedProjects.has(group.project.id)
    for (const conversation of group.items) {
      const open = conversation.id === openId
      if (projectFolded && !open)
        continue
      lines.push(
        {
          kind: 'conversation',
          id: conversation.id,
          projectId: group.project.id,
          hidden: projectsFolded && !open,
        }
      )
    }
  }
  return lines
}

export function entryIndex(entries: readonly ListEntry[], id: string | null): number {
  return id === null ? -1 : entries.findIndex((entry) => entry.id === id)
}

/** The conversation ids between two entries inclusive, in display order; project headers are skipped. */
export function conversationIdsBetween(
  entries: readonly ListEntry[],
  fromId: string,
  toId: string
): string[] {
  const from = entryIndex(entries, fromId)
  const to = entryIndex(entries, toId)
  if (from < 0 || to < 0)
    return []
  const [start, end] = from <= to ? [from, to] : [to, from]
  return entries.slice(start, end + 1).flatMap(
    (entry) => (entry.kind === 'conversation' ? [entry.id] : [])
  )
}

export function conversationIds(entries: readonly ListEntry[]): string[] {
  return entries.flatMap((entry) => (entry.kind === 'conversation'
    ? [entry.id]
    : []))
}

/** The entry `delta` lines away, clamped to the ends; from nowhere, the first or last line. */
export function stepEntry(
  entries: readonly ListEntry[],
  fromId: string | null,
  delta: number
): ListEntry | undefined {
  if (entries.length === 0)
    return undefined
  const index = entryIndex(entries, fromId)
  if (index < 0)
    return delta > 0 ? entries[0] : entries[entries.length - 1]
  return entries[Math.min(entries.length - 1, Math.max(0, index + delta))]
}
