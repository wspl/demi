import type { SidebarConversation, SidebarProject } from './types'

export interface ProjectGroup {
  project: SidebarProject
  items: SidebarConversation[]
}

/** Pinned first, preserving the supplied manual order within each partition. */
export function sortConversations(
  conversations: readonly SidebarConversation[],
): SidebarConversation[] {
  return [...conversations].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
    return 0
  })
}

/** Conversations that run in no project. */
export function plainConversations(
  conversations: readonly SidebarConversation[],
): SidebarConversation[] {
  return sortConversations(conversations.filter((conversation) => conversation.projectId === null))
}

/**
 * Every project in the supplied project order. A project with no
 * conversations still lists, so an opened checkout has a place before its first turn.
 */
export function projectGroups(
  projects: readonly SidebarProject[],
  conversations: readonly SidebarConversation[],
): ProjectGroup[] {
  return projects.map((project) => {
    const items = sortConversations(
      conversations.filter((conversation) => conversation.projectId === project.id),
    )
    return { project, items }
  })
}
