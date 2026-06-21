import type {
  Block,
  ModelSelection,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
  UserContentBlock,
} from '@demi/core'
import type { BashAuditEvent, OutputSnapshot } from '@demi/shell'

export interface ProviderConfig {
  type: string
  config?: unknown
  model: ModelSelection
}

export type ClientFrame =
  | { type: 'open'; provider: ProviderConfig; cwd: string }
  | { type: 'send'; content: UserContentBlock[] }
  | { type: 'set_provider'; provider: ProviderConfig }
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
  | { type: 'tool_progress'; toolUseId: string; output: ToolResultContentBlock[] }
  | { type: 'shell_output'; shellId: string; snapshot: OutputSnapshotLike }
  | { type: 'shell_input_result'; shellId: string; output: ToolResultContentBlock[] }
  | { type: 'audit'; events: BashAuditEvent[] }
  | { type: 'rejected'; command: string; reason: string }
  | { type: 'error'; message: string; code?: string }
  | { type: 'opened' }
  | { type: 'closed' }
