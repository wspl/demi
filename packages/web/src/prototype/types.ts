import type { FileBrowserPlatform } from '@demicodes/web-ui/files/types'
import type { Block, ThinkingConfig, QueuedMessage } from '@demicodes/core'
import type { SidebarConversation, SidebarProject } from '@demicodes/web-ui/sidebar/types'
import type { PendingSteerMessage } from '@demicodes/web-ui/agent/types'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'

export interface DraftFile {
  id: string
  name: string
  src?: string
  destination: 'message' | 'workspace'
}
export interface Conversation extends SidebarConversation {
  archived: boolean
  blocks: Block[]
  draft: string
  files: DraftFile[]
  queue: QueuedMessage[]
  /** Steers accepted while a turn runs; they join the transcript when the current output ends. */
  pendingSteers: PendingSteerMessage[]
  providerId: string
  modelId: string
  thinking: ThinkingConfig
  serviceTierId: string | null
  attachedHosts: { deviceId: string; name: string; cwd: string }[]
  stream: { blockId: string; remaining: string; fail: boolean } | null
  /** Output left unstreamed by Stop; Resume continues it in a fresh block. */
  paused: string | null
}
export interface Device {
  id: string
  name: string
  online: boolean
  platform: FileBrowserPlatform
  home: string
}
export interface Project extends SidebarProject {
  deviceId: string
  branch: string | null
}
export interface PrototypeProvider extends ProviderInfo {
  models: ModelInfo[]
}
