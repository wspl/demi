// Browser-safe client surface. Importing this entry must not pull AgentServer,
// AgentSession, or @demicodes/shell into a frontend bundle.

export { AgentClient, type AgentClientListener } from './client'
export { createWebSocketClientTransport, createWebSocketServerTransport, type JsonWebSocket } from './websocket-transport'
export type { AgentTransport, AgentClientTransport, AgentServerTransport } from './transport'
export type {
  ClientFrame,
  ServerFrame,
  ClientSessionEvent,
  TranscriptPatch,
  ShellCommandStatusLike,
} from './frames'
export type { ProviderSelection } from '@demicodes/provider'
