// Browser-safe client surface. Importing this entry must not pull AgentServer,
// AgentSession, or @demicodes/shell into a frontend bundle.

export { AgentClient, EditRejectedError, type AgentClientListener } from './client/client'
export { ProviderStreamError } from './session/provider-stream-error'
export type { AgentActionOptions } from './client/client'
export type { AgentMetadata } from './types'
export {
  createWebSocketClientTransport,
  createWebSocketServerTransport,
  type JsonWebSocket
} from './protocol/websocket-transport'
export type {
  AgentTransport,
  AgentClientTransport,
  AgentServerTransport
} from './protocol/transport'
export type {
  ClientFrame,
  ServerFrame,
  ClientSessionEvent,
  TranscriptPatch,
  ShellCommandStatusLike,
} from './protocol/frames'
export type { ProviderSelection } from '@demicodes/provider'
export {
  modelSelectionSchema,
  thinkingConfigSchema,
  editRequestSchema,
  editResultSchema,
  transcriptVersionSchema,
  serverFrameSchema,
  blockSchema,
  transcriptPatchSchema,
  shellCommandStatusSchema,
  toolResultContentBlockSchema,
  refSourceSchema,
  refBase64SourceSchema,
  type EditRequest,
  type TranscriptVersion,
  type RefSource,
  type RefBase64Source,
} from './protocol/schemas'
export { shellToolViewSchema } from './server/summaries'
export { isEditableUserMessage } from './transcript/user-message'
export { applyTranscriptPatches } from './transcript/patch'

export { agentMessageSchema } from './protocol/agent-message'
