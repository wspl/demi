import type {
  Block,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
  UserContentBlock,
} from '@demi/core'
import type { ProviderSelection } from '@demi/provider'
import type { BashAuditEvent, OutputSnapshot } from '@demi/shell'

export type ClientFrame =
  | { type: 'open'; provider: ProviderSelection; cwd: string }
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
  | { type: 'shell_input'; shellId: string; stdin: string }
  | { type: 'close' }

export type ServerFrame =
  | { type: 'opened' }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'transcript_snapshot'; blocks: Block[] }
  | { type: 'transcript_patch'; patches: TranscriptPatch[] }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'steer_result'; steerId: string; status: 'accepted' }
  | { type: 'steer_result'; steerId: string; status: 'rejected'; reason: string }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; snapshot: OutputSnapshotLike }
  | { type: 'shell_input_result'; shellId: string; output: ToolResultContentBlock[] }
  | { type: 'audit'; events: BashAuditEvent[] }
  | { type: 'error'; message: string; code?: string }
  | { type: 'closed' }

export type TranscriptPatch =
  | { op: 'add'; path: ['blocks', number]; value: Block }
  | { op: 'remove'; path: ['blocks', number] }
  | { op: 'replace'; path: ['blocks']; value: Block[] }

export type OutputSnapshotLike = OutputSnapshot

export type ClientSessionEvent =
  | { type: 'transcript_snapshot'; blocks: Block[] }
  | { type: 'transcript_patch'; patches: TranscriptPatch[]; blocks: Block[] }
  | { type: 'phase'; phase: SessionPhase }
  | { type: 'queue'; queue: QueuedMessage[] }
  | { type: 'steer_result'; steerId: string; status: 'accepted' }
  | { type: 'steer_result'; steerId: string; status: 'rejected'; reason: string }
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; snapshot: OutputSnapshotLike }
  | { type: 'shell_input_result'; shellId: string; output: ToolResultContentBlock[] }
  | { type: 'audit'; events: BashAuditEvent[] }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'error'; message: string; code?: string }
  | { type: 'opened' }
  | { type: 'closed' }
