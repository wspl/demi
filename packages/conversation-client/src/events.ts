import type {
  AgentEntry,
  Block,
  BlockVersions,
  ConversationAttachment,
  InstructionEntry,
  PendingSteer,
  ProviderFailureFacts,
  QueuedMessage,
  ReplyBlockFacts,
  ServerFrame,
  TranscriptPatch,
  Version,
} from '@demicodes/protocol'

export type ServerFrameOf<Type extends ServerFrame['type']> = Extract<ServerFrame, { type: Type }>

/**
 * The output of the request a node's agent runs, as the page holds it.
 * Its times are the page's clock, `Date.now()`: the frame's arrival less how
 * long the request had streamed, so no time of the server's is compared with
 * the page's.
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
 * server frame as it arrived. The agents and a node's `transcript_reset` are
 * what the client read, which it tells as soon as the read answers, before
 * the lane opens.
 * The queue and the pending steers are the root's agent entry's, told as they
 * change and with each `agents` event. The transcript events carry what the
 * client read or a frame brought of a node, which the page applies to the
 * parts of that node's transcript it holds.
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
  | {
      /** Every agent of the conversation, as a read of it lists them, at `version`. */
      type: 'agents'
      agents: AgentEntry[]
      version: Version
      /**
       * When it arrived, as `Date.now()`: an agent waits for its model since
       * then less its `waitedMs`, so no time of the server's is compared with
       * the page's clock.
       */
      receivedAt: number
    }
  | (ServerFrameOf<'agent'> & { receivedAt: number })
  | {
      /** The node's latest page, from `start` on. */
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
      /** The descriptions of the attachments `blocks` name. */
      attachments: ConversationAttachment[]
      /** The facts of the plugins' reply blocks `blocks` name. */
      replyBlocks: ReplyBlockFacts[]
      /** The entries of the agent's newest instructions block. */
      instructions: InstructionEntry[]
      /** The summary size of each compaction marker's boundary. */
      summaries: { marker: string; summaryTokens: number }[]
      /** The index from which the page held blocks stand as they were: the read starts there; undefined when none can be vouched for. */
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
      /** The descriptions of the attachments the patches name. */
      attachments: ConversationAttachment[]
      /** The facts of the plugins' reply blocks the patches name. */
      replyBlocks: ReplyBlockFacts[]
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
      node: string
      /** The output of the request the node's agent runs; null when its turn has none. */
      output: RequestOutputReading | null
    }
  | {
      /** The connection ended; the client does nothing more. */
      type: 'disconnected'
      error: Error
    }

export type ConversationClientListener = (event: ClientSessionEvent) => void
