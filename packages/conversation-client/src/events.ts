import type {
  Block,
  BlockVersions,
  ConversationAttachment,
  PendingSteer,
  ProviderFailureFacts,
  QueuedMessage,
  ServerFrame,
  TranscriptPatch,
  Version,
} from '@demicodes/protocol'

export type ServerFrameOf<Type extends ServerFrame['type']> = Extract<ServerFrame, { type: Type }>

/**
 * The output of the request the root agent runs, as the page holds it
 * (`docs/decisions/0012-live-output-tokens.md`). Its times are the page's
 * clock, `Date.now()`: the frame's arrival less how long the request had
 * streamed, so no time of the server's is compared with the page's.
 */
export interface RequestOutputReading {
  /** Another id is another request, whose count starts from zero. */
  requestId: string
  /** An estimate while it streams, then the response's own count. */
  tokens: number
  /** When the request's first token came; null before it. */
  firstTokenAt: number | null
  /** When its response ended; null while it streams. */
  endedAt: number | null
}

/** The facts the backend read out of error blocks, by block id. */
export type Failures = Record<string, ProviderFailureFacts>

/**
 * What a `ConversationClient` tells its listeners. Most events are the
 * server frame as it arrived. The queue and the pending steers are the root's
 * agent entry's, told as they change and with each `agents` frame. The transcript events carry what the frame
 * brought of a node, which the page applies to the parts of that node's
 * transcript it holds; a patch carries no revision, which the client keeps.
 */
export type ClientSessionEvent =
  | ServerFrameOf<'opened'>
  | ServerFrameOf<'closed'>
  | ServerFrameOf<'edit_result'>
  | ServerFrameOf<'rejected'>
  | ServerFrameOf<'steer_result'>
  | ServerFrameOf<'send_result'>
  | ServerFrameOf<'remove_result'>
  | ServerFrameOf<'abort_result'>
  | ServerFrameOf<'shell_output'>
  | ServerFrameOf<'cache_chart'>
  | ServerFrameOf<'shell_write_result'>
  | ServerFrameOf<'retry_scheduled'>
  | ServerFrameOf<'error'>
  | ServerFrameOf<'pong'>
  | (ServerFrameOf<'agents'> & {
      /**
       * When it arrived, as `Date.now()`: an agent waits for its model since
       * then less its `waitedMs`, so no time of the server's is compared with
       * the page's clock.
       */
      receivedAt: number
    })
  | (ServerFrameOf<'agent'> & { receivedAt: number })
  | {
      /** The node's blocks from `start` on. */
      type: 'transcript_reset'
      node: string
      start: number
      length: number
      blocks: Block[]
      version: Version
      /** The versions of its blocks that have others. */
      versions: BlockVersions[]
      /** The facts of the error blocks in `blocks`. */
      failures: Failures
      /** The descriptions of the attachments `blocks` name that the node's replies show. */
      attachments: ConversationAttachment[]
      /** The index the client named when it subscribed; a reset elsewhere means a rewrite. */
      asked: number | undefined
    }
  | {
      type: 'transcript_patch'
      node: string
      patches: TranscriptPatch[]
      /** The node's transcript's version after them. */
      version: Version
      /** The facts of the error blocks the patches bring. */
      failures: Failures
      /** The descriptions of the attachments the patches name that the node's replies show. */
      attachments: ConversationAttachment[]
    }
  | {
      type: 'queue'
      /** The root's queued messages, as its agent entry lists them. */
      queue: QueuedMessage[]
    }
  | {
      type: 'starting'
      /** The message the root's running turn started with, until its turn writes it; null otherwise. */
      starting: QueuedMessage | null
    }
  | {
      type: 'pending_steers'
      /** The accepted steers not yet in the transcript, as the root's agent entry lists them. */
      pendingSteers: PendingSteer[]
    }
  | {
      type: 'request_output'
      /** The output of the request the root agent runs; null when its turn has none. */
      output: RequestOutputReading | null
    }
  | {
      /** The connection ended; the client does nothing more. */
      type: 'disconnected'
      error: Error
    }

export type ConversationClientListener = (event: ClientSessionEvent) => void
