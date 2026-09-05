import type { Block, ThinkingConfig, QueuedMessage } from '@demicodes/core'
import type { SidebarConversation, SidebarProject } from '@demicodes/web-ui/sidebar/types'
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
  providerId: string
  modelId: string
  thinking: ThinkingConfig
  serviceTierId: string | null
  attachedHosts: string[]
  stream: { blockId: string; remaining: string; fail: boolean } | null
}
export interface Device {
  id: string
  name: string
  online: boolean
}
export interface Project extends SidebarProject {
  deviceId: string
  branch: string | null
  branches: string[]
}
export interface PrototypeProvider extends ProviderInfo {
  models: ModelInfo[]
}
