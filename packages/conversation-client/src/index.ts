// The browser's client of the conversation socket. It validates every frame it
// receives with the schemas of `@demicodes/protocol`, and imports no user
// interface package.
//
// `applyTranscriptPatches` is the one transcript patch applier. The agent's
// Rust tests write patch sequences together with the snapshot each must
// produce, and this package's tests apply them, so no second applier exists in
// Rust.

export { ActionOutlivedError, ConversationClient, EditRejectedError, RefusedError, SavedGoneError, SessionError, SteerRejectedError, type ConversationClientOptions } from './client'
export type { ConversationClientListener, ClientSessionEvent, Failures, RequestOutputReading, ServerFrameOf } from './events'
export {
  EMPTY_TRANSCRIPT,
  heldTranscript,
  addPage,
  addWindow,
  applyTranscriptPatches,
  heldBlocks,
  heldEdge,
  latestWindow,
  resetTranscript,
  windowEnd,
  windowOf,
  withWholeBlock,
  type Held,
  type HeldBlock,
  type HeldTranscript,
  type TextArrival,
  type TranscriptWindow,
} from './transcript'
export {
  createWebSocketTransport,
  type ConversationClientTransport,
  type SocketClose,
  type SocketMessage,
  type WebSocketLike,
} from './transport'
