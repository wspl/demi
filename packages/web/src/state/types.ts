import type { Block, ProviderFailureFacts, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import type { FileBrowserPlatform } from '@demicodes/web-ui/files/types'
import type {
  SidebarConversation,
  SidebarProject,
} from '@demicodes/web-ui/sidebar/types'
import type { ModelIntent, PendingSteerMessage } from '@demicodes/web-ui/agent/types'
import type {
  ComposerFileAttachment,
  ComposerRemoteAttachment,
} from '@demicodes/web-ui/agent/message-input/attachments'
import type { PendingAction } from '@demicodes/web-ui/agent/activity-slot'
import type { SessionLoad } from '@demicodes/web-ui/agent/session-status'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { ConversationTarget, DeviceKind } from '../api/generated/web-api'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import type { SavedDraft, SavedFile } from '../conversation/drafts'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'

export type ProductAttachment =
  | (ComposerFileAttachment & Pick<SavedFile, 'file' | 'upload'>)
  | (ComposerRemoteAttachment & { deviceId: string })

export interface Conversation extends SidebarConversation {
  persistence: 'draft' | 'pending' | 'synced'
  target: ConversationTarget
  contextVersion: number
  revision: number
  readRevision: number
  archived: boolean
  /** No message is newer than the last generated title, and whether a title request is running. */
  titleCurrent: boolean
  titleGenerating: boolean
  createdAt: string
  cwd: string
  blocks: Block[]
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  model: ModelIntent
  lastError: string | null
  draft: string
  /** Every file the composer carries, those of the message first, in its order. */
  files: ProductAttachment[]
  /** The files the message has, in the order of its capsules; the composer's document says so. */
  attachmentIds: string[]
  submission: 'idle' | 'sending'
  pendingSend: SavedDraft['pendingSend']
  messageEdit: MessageEditState | null
  scroll: PersistedScrollState | null
  attachedHosts: {
    deviceId: string
    name: string
    cwd: string | null
    online: boolean
  }[]
  subagents: SubagentRecord[]
  terminals: TerminalRecord[]
  load: SessionLoad
  pendingAction: PendingAction
  /** The failure facts of the error blocks, by block id, as the backend sends them. */
  failures: Record<string, ProviderFailureFacts>
}

export interface Device {
  id: string
  kind: DeviceKind
  name: string
  online: boolean
  platform: FileBrowserPlatform
  home: string | null
  seen?: string
}

export interface Project extends SidebarProject {
  deviceId: string
}
