import type {
  Block,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
  UserContentBlock,
} from '@demicodes/core'
import type { ProviderSelection } from '@demicodes/provider'
import type { AbortResult } from './types'
import type { BashAuditEvent, ShellCommandSnapshot } from '@demicodes/shell'

/** A persisted conversation in a workspace (cwd), for the resume/history list. */
export interface ConversationSummary {
  id: string
  title: string
  createdAt: string
  updatedAt: string
}

export type ClientFrame =
  | { type: 'open'; provider: ProviderSelection; cwd: string; sessionId: string }
  | { type: 'send'; messageId: string; content: UserContentBlock[] }
  | { type: 'dequeue_message'; messageId: string }
  | { type: 'send_queued_message'; messageId: string }
  | { type: 'steer_queued_message'; messageId: string; steerId: string }
  | { type: 'clear_message_queue' }
  | { type: 'steer'; steerId: string; content: UserContentBlock[] }
  | { type: 'cancel_pending_steer'; steerId: string }
  | { type: 'set_provider'; provider: ProviderSelection }
  | { type: 'abort' }
  | { type: 'retry' }
  | { type: 'resume' }
  | { type: 'compact' }
  | { type: 'shell_write'; commandId: string; stdin: string }
  | { type: 'list_conversations'; cwd: string }
  // Requests a fresh transcript_snapshot; sent by the client when it detects a
  // revision gap in the patch stream (defensive resync, transports are ordered).
  | { type: 'sync_transcript' }
  | { type: 'close' }

export type ServerFrame =
  | { type: 'opened' }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'transcript_snapshot'; blocks: Block[]; revision: number }
  | { type: 'transcript_patch'; patches: TranscriptPatch[]; revision: number }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'steer_result'; steerId: string; status: 'accepted' }
  | { type: 'steer_result'; steerId: string; status: 'rejected'; reason: string }
  | { type: 'abort_result'; result: AbortResult }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; commandId: string; snapshot: ShellCommandSnapshotLike }
  | { type: 'shell_write_result'; commandId: string; output: ToolResultContentBlock[] }
  | { type: 'audit'; events: BashAuditEvent[] }
  | { type: 'conversations'; conversations: ConversationSummary[] }
  | { type: 'error'; message: string; code?: string }
  | { type: 'closed' }

/**
 * Wire patches for transcript replication. Produced directly by the Transcript's
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

export type ShellCommandSnapshotLike = ShellCommandSnapshot

export type ClientSessionEvent =
  | { type: 'transcript_snapshot'; blocks: Block[] }
  | { type: 'transcript_patch'; patches: TranscriptPatch[]; blocks: Block[] }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'steer_result'; steerId: string; status: 'accepted' }
  | { type: 'steer_result'; steerId: string; status: 'rejected'; reason: string }
  | { type: 'abort_result'; result: AbortResult }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; commandId: string; snapshot: ShellCommandSnapshotLike }
  | { type: 'shell_write_result'; commandId: string; output: ToolResultContentBlock[] }
  | { type: 'audit'; events: BashAuditEvent[] }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'error'; message: string; code?: string }
  | { type: 'opened' }
  | { type: 'closed' }
