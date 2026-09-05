import type { ConversationStatus } from '@demicodes/web-ui/agent/conversation-status'

/** A checkout the agent works in, on the host that has it. Conversations that belong to one run there. */
export interface SidebarProject {
  id: string
  name: string
  /** The machine the checkout lives on; the row shows it beside the name. */
  host: string
  hostKind: 'device' | 'cloud'
  path: string
}

export interface SidebarConversation {
  id: string
  title: string
  /** Last activity metadata; display order follows the supplied array. */
  updatedAt: string
  status: ConversationStatus
  /** Null for a plain conversation that runs in no project. */
  projectId: string | null
  pinned: boolean
  /** Finished while the user was elsewhere and not opened since. */
  unread: boolean
}

/** One installed extension entry: a plugin (tools and integrations) or a skill (a packaged workflow). */
export interface SidebarExtension {
  id: string
  name: string
  summary: string
  enabled: boolean
}

export interface SidebarAccount {
  name: string
  email: string
  plan: string
}


/** Reordering stays within a project and pin partition; it never changes execution bindings. */
export interface SidebarReorder {
  kind: 'project' | 'conversation'
  id: string
  beforeId: string | null
}
