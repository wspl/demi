import type { ProjectGroup } from './group-conversations'
import type { SidebarConversation } from './sidebar-data'

/** One navigable line of the list, in display order: a conversation row or a project header. */
export type ListEntry =
  | { kind: 'conversation'; id: string; projectId: string | null }
  | { kind: 'project'; id: string }

/** The lines the keyboard can reach: plain rows, then each project header and, unless folded, its rows. */
export function visibleEntries(
  plain: readonly SidebarConversation[],
  groups: readonly ProjectGroup[],
  folded: ReadonlySet<string>,
): ListEntry[] {
  const entries: ListEntry[] = plain.map((conversation) => ({ kind: 'conversation', id: conversation.id, projectId: null }))
  for (const group of groups) {
    entries.push({ kind: 'project', id: group.project.id })
    if (folded.has(group.project.id)) continue
    for (const conversation of group.items) {
      entries.push({ kind: 'conversation', id: conversation.id, projectId: group.project.id })
    }
  }
  return entries
}

export function entryIndex(entries: readonly ListEntry[], id: string | null): number {
  return id === null ? -1 : entries.findIndex((entry) => entry.id === id)
}

/** The conversation ids between two entries inclusive, in display order; project headers are skipped. */
export function conversationIdsBetween(entries: readonly ListEntry[], fromId: string, toId: string): string[] {
  const from = entryIndex(entries, fromId)
  const to = entryIndex(entries, toId)
  if (from < 0 || to < 0) return []
  const [start, end] = from <= to ? [from, to] : [to, from]
  return entries.slice(start, end + 1).flatMap((entry) => (entry.kind === 'conversation' ? [entry.id] : []))
}

export function conversationIds(entries: readonly ListEntry[]): string[] {
  return entries.flatMap((entry) => (entry.kind === 'conversation' ? [entry.id] : []))
}

/** The entry `delta` lines away, clamped to the ends; from nowhere, the first or last line. */
export function stepEntry(entries: readonly ListEntry[], fromId: string | null, delta: number): ListEntry | undefined {
  if (entries.length === 0) return undefined
  const index = entryIndex(entries, fromId)
  if (index < 0) return delta > 0 ? entries[0] : entries[entries.length - 1]
  return entries[Math.min(entries.length - 1, Math.max(0, index + delta))]
}
