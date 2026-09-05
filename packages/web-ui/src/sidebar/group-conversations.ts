import type { SidebarConversation, SidebarProject } from './types'

export interface ProjectGroup {
  project: SidebarProject
  items: SidebarConversation[]
  /** Latest activity in the group; projects order by it. */
  updatedAt: string
}

/** Pinned first, then most recent first. */
export function sortConversations(conversations: readonly SidebarConversation[]): SidebarConversation[] {
  return [...conversations].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
    return b.updatedAt.localeCompare(a.updatedAt)
  })
}

/** Conversations that run in no project. */
export function plainConversations(conversations: readonly SidebarConversation[]): SidebarConversation[] {
  return sortConversations(conversations.filter((conversation) => conversation.projectId === null))
}

/**
 * Every project with its conversations, most recently active project first. A project with no
 * conversations still lists, so an opened checkout has a place before its first turn.
 */
export function projectGroups(projects: readonly SidebarProject[], conversations: readonly SidebarConversation[]): ProjectGroup[] {
  return projects
    .map((project) => {
      const items = sortConversations(conversations.filter((conversation) => conversation.projectId === project.id))
      return { project, items, updatedAt: items.reduce((latest, item) => (item.updatedAt > latest ? item.updatedAt : latest), '') }
    })
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
}
