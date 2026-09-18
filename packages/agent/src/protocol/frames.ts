import type { z } from 'zod'
import type {
  Block,
  ProviderErrorDiagnostics,
  ProviderFailureFacts,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
} from '@demicodes/core'
import type { AbortResult } from '../types'
import type { ShellCommandStatus } from '@demicodes/shell'
import type {
  clientFrameSchema,
  editResultSchema,
  pendingSteersFrameSchema,
  serverFrameSchema,
  subagentJobSchema,
  transcriptPatchSchema,
} from './schemas'

/** One child agent session as seen on the parent connection. */
export type SubagentJob = z.infer<typeof subagentJobSchema>

/**
 * The inbound frames, derived from their zod declaration in `schemas.ts` —
 * the schema is the single source of truth for this union, and the server
 * validates every arriving frame against it at transport ingress.
 */
export type ClientFrame = z.infer<typeof clientFrameSchema>

/**
 * The outbound frames, derived the same way: the client validates every
 * arriving frame against `serverFrameSchema` before acting on it.
 */
export type ServerFrame = z.infer<typeof serverFrameSchema>

/**
 * Wire patches for transcript replication. Produced directly by the
 * TranscriptLog's
 * mutation journal (never diff-derived). `append_text` carries streaming deltas
 * for the `text` field of the block at the index (text/thinking blocks), keeping
 * per-delta cost O(delta) instead of O(block) or O(transcript).
 */
export type TranscriptPatch = z.infer<typeof transcriptPatchSchema>

export type ShellCommandStatusLike = ShellCommandStatus

export type ClientSessionEvent =
  | z.infer<typeof editResultSchema>
  | {
      type: 'transcript_reset';
      blocks: Block[]
      /** The facts of every error block in `blocks` the host read, by block id. */
      failures: Record<string, ProviderFailureFacts>
    }
  | {
      type: 'transcript_patch';
      patches: TranscriptPatch[];
      blocks: Block[]
      failures: Record<string, ProviderFailureFacts>
    }
  | {
      type: 'phase';
      phase: SessionPhase
    }
  | {
      type: 'queue';
      queue: QueuedMessage[]
    }
  | z.infer<typeof pendingSteersFrameSchema>
  | {
      type: 'steer_result';
      steerId: string;
      status: 'accepted'
    }
  | {
      type: 'steer_result';
      steerId: string;
      status: 'rejected';
      reason: string
    }
  | {
      type: 'abort_result';
      result: AbortResult
    }
  | {
      type: 'tool_progress';
      toolUseId: string;
      output: ToolResultContentBlock[]
    }
  | {
      type: 'shell_output';
      shellId: string;
      commandId: string;
      status: ShellCommandStatusLike
    }
  | {
      type: 'shell_write_result';
      commandId: string;
      output: ToolResultContentBlock[]
    }
  | {
      type: 'retry_scheduled'
      attempt: number
      delayMs: number
      code: string | null
      diagnostics?: ProviderErrorDiagnostics
    }
  | {
      type: 'rejected';
      command: string;
      reason: string
    }
  | {
      type: 'error';
      message: string;
      code?: string;
      diagnostics?: ProviderErrorDiagnostics
    }
  | {
      type: 'subagent';
      event: 'started' | 'closed';
      job: SubagentJob
    }
  | {
      type: 'subagent_transcript_reset';
      subagentId: string;
      blocks: Block[]
      /** The facts for this frame's error blocks, by block id. */
      failures: Record<string, ProviderFailureFacts>
    }
  | {
      type: 'subagent_transcript_patch';
      subagentId: string;
      patches: TranscriptPatch[]
      /** The facts for the error blocks this patch adds, by block id. */
      failures: Record<string, ProviderFailureFacts>
    }
  | { type: 'opened' }
  | { type: 'closed' }
  | { type: 'disconnected' }
