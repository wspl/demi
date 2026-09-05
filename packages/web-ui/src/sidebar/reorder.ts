import type { SidebarConversation, SidebarProject, SidebarReorder } from './types'

export function reorderPeers(
  source: Pick<SidebarReorder, 'kind' | 'id'>,
  projects: readonly SidebarProject[],
  conversations: readonly SidebarConversation[],
): string[] {
  if (source.kind === 'project') return projects.map((project) => project.id)
  const item = conversations.find((conversation) => conversation.id === source.id)
  if (!item) return []
  return conversations
    .filter(
      (conversation) =>
        conversation.projectId === item.projectId && conversation.pinned === item.pinned,
    )
    .map((conversation) => conversation.id)
}

export function sidebarDrop(
  source: Pick<SidebarReorder, 'kind' | 'id'>,
  target: Pick<SidebarReorder, 'kind' | 'id'>,
  after: boolean,
  projects: readonly SidebarProject[],
  conversations: readonly SidebarConversation[],
): SidebarReorder | null {
  if (source.kind !== target.kind || source.id === target.id) return null
  const peers = reorderPeers(source, projects, conversations)
  if (!peers.includes(source.id) || !peers.includes(target.id)) return null
  const remaining = peers.filter((id) => id !== source.id)
  const index = remaining.indexOf(target.id) + Number(after)
  const beforeId = remaining[index] ?? null
  if ((peers[peers.indexOf(source.id) + 1] ?? null) === beforeId) return null
  return { ...source, beforeId }
}
