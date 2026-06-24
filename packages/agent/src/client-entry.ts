// Browser-safe client surface. Importing this entry must not pull AgentServer,
// AgentSession, or @demi/shell into a frontend bundle.

export { AgentClient, type AgentClientListener } from './client'
export { createWebSocketClientTransport, createWebSocketServerTransport, type JsonWebSocket } from './websocket-transport'
export type { AgentTransport, AgentClientTransport, AgentServerTransport } from './transport'
export type {
  ClientFrame,
  ServerFrame,
  ClientSessionEvent,
  TranscriptPatch,
  OutputSnapshotLike,
} from './frames'
export type { ProviderSelection } from '@demi/provider'
