import type { Block, PendingCall, ProviderFailureFacts, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import type { FileBrowserPlatform } from '@demicodes/web-ui/files/types'
import type { DeviceStart } from '@demicodes/web-ui/devices/installation'
import type { DeviceState } from '@demicodes/web-ui/devices/state'
import type { DeviceReport } from '@demicodes/web-ui/devices/report'
import type {
  SidebarConversation,
  SidebarProject,
} from '@demicodes/web-ui/sidebar/types'
import type { ConversationState, ModelSettings, PendingSteerMessage } from '@demicodes/web-ui/agent/types'
import type {
  ComposerFileAttachment,
  ComposerRemoteAttachment,
} from '@demicodes/web-ui/agent/message-input/attachments'
import type { PendingAction } from '@demicodes/web-ui/agent/activity-slot'
import type { SessionLoad } from '@demicodes/web-ui/agent/session-status'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { ConversationDraft, ConversationTarget, DeviceKind } from '../api/generated/web-api'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import type { SavedDraft, SavedFile } from '../conversation/drafts'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'

export type ProductAttachment =
  | (ComposerFileAttachment & Pick<SavedFile, 'file' | 'upload'>)
  | (ComposerRemoteAttachment & { deviceId: string })

/** The sidebar's `device` is derived from `target` and the devices for the row (`App.vue`), never kept. */
export interface Conversation extends Omit<SidebarConversation, 'device'> {
  persistence: 'draft' | 'pending' | 'synced'
  target: ConversationTarget
  revision: number
  readRevision: number
  archived: boolean
  /** No message is newer than the last generated title, and whether a title request is running. */
  titleCurrent: boolean
  titleGenerating: boolean
  /** The open tree runs with commands of plugins since turned on or off; a reload would change it. */
  pluginsChanged: boolean
  createdAt: string
  cwd: string
  blocks: Block[]
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  /** The calls the model is writing (`runtime.md` § Calls being written). */
  pendingCalls: PendingCall[]
  model: ModelSettings
  lastError: string | null
  /** The composer's Markdown, a mark where each file's capsule stands. */
  draft: string
  /** The draft the backend holds, as this page last read or saved it; null until read. */
  savedDraft: ConversationDraft | null
  /** The revision of the backend's draft that `draft` was built on; null while none was read. */
  draftBase: number | null
  /** Counts the drafts shown from outside, another page's or a restored one, which the editor then shows anew. */
  draftShown: number
  /** Every file the composer carries, those of the message first, in its order. */
  files: ProductAttachment[]
  /** The files the message has, in the order of its capsules; the composer's document says so. */
  attachmentIds: string[]
  pendingSend: SavedDraft['pendingSend']
  messageEdit: MessageEditState | null
  scroll: PersistedScrollState | null
  attachedHosts: {
    deviceId: string
    name: string
    cwd: string | null
  }[]
  /** The hosts revision `attachedHosts` was read at or is being read at; a summary with a higher one reads them again. */
  hostsRevision: number
  subagents: SubagentRecord[]
  terminals: TerminalRecord[]
  load: SessionLoad
  pendingAction: PendingAction
  /** The failure facts of the error blocks, by block id, as the backend sends them. */
  failures: Record<string, ProviderFailureFacts>
  contextUsage: ConversationState['contextUsage']
}

export interface Device extends DeviceReport {
  id: string
  kind: DeviceKind
  name: string
  state: DeviceState
  platform: FileBrowserPlatform
  home: string | null
  seen?: string
  /** How to start a paired device's runner again; null for the Cloud. */
  start: DeviceStart | null
}

export type Project = SidebarProject & {
  deviceId: string
}
