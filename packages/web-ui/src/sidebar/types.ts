import type { Component } from 'vue'
import type { ConversationStatus } from '@demicodes/web-ui/agent/conversation-status'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'

export type { ListLoad }

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

/** The signed-in account the sidebar's foot shows. */
export interface SidebarAccount {
  name: string
  email: string
}

/** An entry of the sidebar's top group that opens a section of the settings dialog. */
export interface SidebarEntry {
  /** The settings section it opens. */
  section: string
  label: string
  icon: Component
}

/** Reordering stays within a project and pin partition; it never changes execution bindings. */
export interface SidebarReorder {
  kind: 'project' | 'conversation'
  id: string
  beforeId: string | null
}
