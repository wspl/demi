import type {
  Block,
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
 * What an `AgentClient` tells its listeners. Most events are the server frame
 * as it arrived; transcript events carry the transcript as the client holds
 * it after the frame, and none carries a revision, which the client keeps.
 */
export type ClientSessionEvent =
  | ServerFrameOf<'opened'>
  | ServerFrameOf<'closed'>
  | ServerFrameOf<'edit_result'>
  | ServerFrameOf<'rejected'>
  | ServerFrameOf<'phase'>
  | ServerFrameOf<'queue'>
  | ServerFrameOf<'steer_result'>
  | ServerFrameOf<'abort_result'>
  | ServerFrameOf<'shell_output'>
  | ServerFrameOf<'shell_write_result'>
  | ServerFrameOf<'retry_scheduled'>
  | ServerFrameOf<'error'>
  | ServerFrameOf<'subagent'>
  | {
      type: 'transcript_reset'
      blocks: Block[]
      /** The facts of the error blocks in `blocks`. */
      failures: Failures
    }
  | {
      type: 'transcript_patch'
      patches: TranscriptPatch[]
      /** The transcript with the patches applied. */
      blocks: Block[]
      /** The facts of every error block since the last reset. */
      failures: Failures
    }
  | {
      type: 'pending_steers'
      /** The accepted steers not yet in the transcript. */
      pendingSteers: PendingSteer[]
    }
  | {
      type: 'subagent_transcript_reset'
      subagentId: string
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

export type AgentClientListener = (event: ClientSessionEvent) => void
