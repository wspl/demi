import type {
  Block,
  PendingCall,
  PendingSteer,
  ProviderFailureFacts,
  ServerFrame,
  TranscriptPatch,
} from '@demicodes/protocol'

/** The server frame of one `type`. */
export type ServerFrameOf<Type extends ServerFrame['type']> = Extract<ServerFrame, { type: Type }>

/** The facts the backend read out of error blocks, by block id. */
export type Failures = Record<string, ProviderFailureFacts>

/**
 * What an `ConversationClient` tells its listeners. Most events are the server frame
 * as it arrived; the transcript events carry what the frame brought, which
 * the page applies to the parts of the transcript it holds, and none
 * carries a revision, which the client keeps.
 */
export type ClientSessionEvent =
  | ServerFrameOf<'opened'>
  | ServerFrameOf<'closed'>
  | ServerFrameOf<'edit_result'>
  | ServerFrameOf<'rejected'>
  | ServerFrameOf<'phase'>
  | ServerFrameOf<'context_usage'>
  | ServerFrameOf<'queue'>
  | ServerFrameOf<'steer_result'>
  | ServerFrameOf<'abort_result'>
  | ServerFrameOf<'shell_output'>
  | ServerFrameOf<'shell_write_result'>
  | ServerFrameOf<'retry_scheduled'>
  | ServerFrameOf<'error'>
  | ServerFrameOf<'subagent'>
  | {
      /** The blocks from `start` on (`runtime.md` § Where a reset starts). */
      type: 'transcript_reset'
      start: number
      length: number
      blocks: Block[]
      /** The facts of the error blocks in `blocks`. */
      failures: Failures
      /** The index the client named in its `open` or `sync_transcript`; a reset elsewhere means a rewrite. */
      asked: number | undefined
    }
  | {
      type: 'transcript_patch'
      patches: TranscriptPatch[]
      /** The facts of the error blocks the patches bring. */
      failures: Failures
    }
  | {
      type: 'pending_steers'
      /** The accepted steers not yet in the transcript. */
      pendingSteers: PendingSteer[]
    }
  | {
      type: 'pending_calls'
      /** The subagent whose calls they are; absent for the root's. */
      subagentId?: string
      /** The calls the model is writing (`runtime.md` § Calls being written). */
      pendingCalls: PendingCall[]
    }
  | {
      type: 'subagent_transcript_reset'
      subagentId: string
      start: number
      length: number
      blocks: Block[]
      failures: Failures
    }
  | {
      /** A patch of the subagent's transcript that follows the last one the client saw. */
      type: 'subagent_transcript_patch'
      subagentId: string
      patches: TranscriptPatch[]
      failures: Failures
    }
  | {
      /** The connection ended; the client does nothing more. */
      type: 'disconnected'
      error: Error
    }

export type ConversationClientListener = (event: ClientSessionEvent) => void
