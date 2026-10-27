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
    if (a.pinned !== b.pinned)
      return a.pinned ? -1 : 1
    return 0
  })
}

/** Conversations that run in no project. */
export function plainConversations(
  conversations: readonly SidebarConversation[],
): SidebarConversation[] {
  return sortConversations(
    conversations.filter((conversation) => conversation.projectId === null)
  )
}

/**
 * The projects that hold a listed conversation, in the supplied project
 * order. A project with none has nothing to show, so it is not listed; the
 * open conversation is always listed, so a new conversation's project shows
 * from the moment it opens.
 */
export function projectGroups(
  projects: readonly SidebarProject[],
  conversations: readonly SidebarConversation[],
): ProjectGroup[] {
  return projects.flatMap((project) => {
    const items = sortConversations(
      conversations.filter((conversation) => conversation.projectId === project.id),
    )
    return items.length > 0 ? [{ project, items }] : []
  })
}
