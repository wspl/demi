import type { Component } from 'vue'
import type { ConversationStatus } from '@demicodes/web-ui/agent/conversation-status'
import type { DeviceState } from '@demicodes/web-ui/devices/state'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'

export type { ListLoad }

/** A checkout the agent works in, on the host that has it. Conversations that belong to one run there. */
export type SidebarProject = {
  id: string
  name: string
  /** The machine the checkout lives on; the row shows it beside the name. */
  host: string
  path: string
} & SidebarProjectHost

/** Where a project's checkout lives: the user's Cloud, which is woken when needed, or a device, which is connected or not. */
export type SidebarProjectHost =
  | { hostKind: 'cloud' }
  | {
    hostKind: 'device'
    /** Whether the device's runner serves it now; the row marks it with a dot. */
    state: DeviceState
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
  /** A permission request waits for the user's decision (`permissions.md` § What the user sees). */
  needsYou?: boolean
  /**
   * The paired device a conversation outside a project runs on, which its row's end shows
   * (`product.md` § Conversations and projects); absent on the Cloud and in a project.
   */
  device?: SidebarConversationDevice
}

/** A conversation's device as its row shows it: by name and state, or as one since removed. */
export type SidebarConversationDevice =
  | { kind: 'paired'; name: string; state: DeviceState }
  | { kind: 'removed' }

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
