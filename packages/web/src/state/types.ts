import type { QueuedMessage, SessionPhase } from '@demicodes/core'
import type { DisplayedBlock as Block } from '@demicodes/web-ui/transport/protocol'
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
import type { BackendConversation } from '../api/contracts'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import type { SavedDraft, SavedFile } from '../conversation/drafts'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'

export type ProductAttachment =
  | (ComposerFileAttachment & Pick<SavedFile, 'file' | 'upload'>)
  | (ComposerRemoteAttachment & { deviceId: string })

export interface Conversation extends SidebarConversation {
  persistence: 'draft' | 'pending' | 'synced'
  target: BackendConversation['target']
  contextVersion: number
  revision: number
  readRevision: number
  archived: boolean
  createdAt: string
  cwd: string
  blocks: Block[]
  phase: SessionPhase
  queue: QueuedMessage[]
  pendingSteers: PendingSteerMessage[]
  model: ModelIntent
  lastError: string | null
  draft: string
  files: ProductAttachment[]
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
}

export interface Device {
  id: string
  name: string
  online: boolean
  platform: FileBrowserPlatform
  home: string | null
  seen?: string
}

export interface Project extends SidebarProject {
  deviceId: string
}
