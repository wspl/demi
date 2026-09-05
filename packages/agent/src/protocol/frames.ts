import type { z } from 'zod'
import type {
  Block,
  ProviderErrorDiagnostics,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
} from '@demicodes/core'
import type { AbortResult, AgentMetadata } from '../types'
import type { ShellCommandStatus } from '@demicodes/shell'
import type { clientFrameSchema } from './schemas'

/** One child agent session as seen on the parent connection. */
export interface SubagentJob {
  subagentId: string
  parentSessionId: string
  description: string
  profile: string | null
  phase: 'running' | 'completed' | 'aborted' | 'error'
  /** Action metadata of the round that spawned the child. */
  metadata: AgentMetadata | null
  /** Present on `closed`: the child's last assistant text, at most 32 KiB. */
  result?: string
}

/**
 * The inbound frames, derived from their zod declaration in `schemas.ts` —
 * the schema is the single source of truth for this union, and the server
 * validates every arriving frame against it at transport ingress.
 */
export type ClientFrame = z.infer<typeof clientFrameSchema>

export type ServerFrame =
  | { type: 'opened' }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'transcript_reset'; blocks: Block[]; revision: number }
  | { type: 'transcript_patch'; patches: TranscriptPatch[]; revision: number }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'steer_result'; steerId: string; status: 'accepted' }
  | { type: 'steer_result'; steerId: string; status: 'rejected'; reason: string }
  | { type: 'abort_result'; result: AbortResult }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; commandId: string; status: ShellCommandStatusLike }
  | { type: 'shell_write_result'; commandId: string; output: ToolResultContentBlock[] }
  // A transient provider failure is being retried with backoff; informational.
  | {
      type: 'retry_scheduled'
      attempt: number
      delayMs: number
      code: string | null
      diagnostics?: ProviderErrorDiagnostics
    }
  | { type: 'error'; message: string; code?: string; diagnostics?: ProviderErrorDiagnostics }
  | { type: 'subagent'; event: 'started' | 'closed'; job: SubagentJob }
  | { type: 'subagent_transcript_reset'; subagentId: string; blocks: Block[]; revision: number }
  | { type: 'subagent_transcript_patch'; subagentId: string; patches: TranscriptPatch[]; revision: number }
  | { type: 'closed' }

/**
 * Wire patches for transcript replication. Produced directly by the TranscriptLog's
 * mutation journal (never diff-derived). `append_text` carries streaming deltas
 * for the `text` field of the block at the index (text/thinking blocks), keeping
 * per-delta cost O(delta) instead of O(block) or O(transcript).
 */
export type TranscriptPatch =
  | { op: 'add'; path: ['blocks', number]; value: Block }
  | { op: 'remove'; path: ['blocks', number] }
  | { op: 'replace_block'; path: ['blocks', number]; value: Block }
  | { op: 'append_text'; path: ['blocks', number]; delta: string }
  | { op: 'replace'; path: ['blocks']; value: Block[] }

export type ShellCommandStatusLike = ShellCommandStatus

export type ClientSessionEvent =
  | { type: 'transcript_reset'; blocks: Block[] }
  | { type: 'transcript_patch'; patches: TranscriptPatch[]; blocks: Block[] }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'steer_result'; steerId: string; status: 'accepted' }
  | { type: 'steer_result'; steerId: string; status: 'rejected'; reason: string }
  | { type: 'abort_result'; result: AbortResult }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; commandId: string; status: ShellCommandStatusLike }
  | { type: 'shell_write_result'; commandId: string; output: ToolResultContentBlock[] }
  | {
      type: 'retry_scheduled'
      attempt: number
      delayMs: number
      code: string | null
      diagnostics?: ProviderErrorDiagnostics
    }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'error'; message: string; code?: string; diagnostics?: ProviderErrorDiagnostics }
  | { type: 'subagent'; event: 'started' | 'closed'; job: SubagentJob }
  | { type: 'subagent_transcript_reset'; subagentId: string; blocks: Block[] }
  | { type: 'subagent_transcript_patch'; subagentId: string; patches: TranscriptPatch[] }
  | { type: 'opened' }
  | { type: 'closed' }
