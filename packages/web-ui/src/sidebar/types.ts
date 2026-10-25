import type { Component } from 'vue'
import type { ConversationStatus } from '@demicodes/web-ui/agent/conversation-status'
import type { DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { DeviceState } from '@demicodes/web-ui/devices/state'
import type { ListLoad } from '@demicodes/web-ui/agent/session-status'

export type { ListLoad }

/** A checkout the agent works in, on the host that has it. Conversations that belong to one run there. */
export type SidebarProject = {
  id: string
  /** The checkout's folder name, which the project goes by. */
  name: string
  path: string
  /** How its conversations keep their messages saved for later: each its own, or one list for the project. */
  savedScope: SavedScope
  /** How many messages the project's own list keeps; 0 while each conversation keeps its own. */
  savedCount: number
} & SidebarProjectHost

/** How a project's conversations keep their messages saved for later. */
export type SavedScope = 'conversation' | 'project'

/** Where a project's checkout lives: the user's Cloud, which is woken when needed, or a device, which is connected or not. */
export type SidebarProjectHost =
  | { hostKind: 'cloud' }
  | {
    hostKind: 'device'
    /** The device the row's end marks, with its state, or as one since removed. */
    device: SidebarDevice
  }

export interface SidebarConversation {
  id: string
  /** Null for the placeholder, which shows as New conversation in the reader's language. */
  title: string | null
  /** Last activity metadata; display order follows the supplied array. */
  updatedAt: string
  status: ConversationStatus
  /** Null for a plain conversation that runs in no project. */
  projectId: string | null
  pinned: boolean
  /** Finished or failed while the user was elsewhere and not opened since. */
  unread: boolean
  /**
   * A permission request waits for the user's decision and has not been read: the yellow dot. A
   * request read but not answered still waits in the conversation, but its row is settled.
   */
  needsYou?: boolean
  /**
   * The paired device a conversation outside a project runs on, which its row's end shows;
   * absent on the Cloud and in a project.
   */
  device?: SidebarDevice
  /** Whether it has schedules, which wake it and which an alarm clock at its row's end marks. */
  scheduled?: boolean
  /** The name git gives the linked worktree it works in, which a mark at its row's end shows. */
  worktree?: string
}

/** A device as a row's mark shows it: by name, state and mark (`shownMark`), or as one since removed. */
export type SidebarDevice =
  | { kind: 'paired'; name: string; state: DeviceState; downloadPercent?: number | null; mark: DeviceMark | null }
  | { kind: 'removed' }

/** The signed-in account the sidebar's foot shows. */
export interface SidebarAccount {
  /** The account id, which the avatar's tone follows. */
  id: string
  name: string
  email: string
}

/** The server's update at the sidebar's foot: a release offered, or an update that runs. */
export type SidebarUpdate = { state: 'available', version: string } | { state: 'running' }

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
