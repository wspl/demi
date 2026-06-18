import type {
  ModelSelection,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
  Transcript as CoreTranscript,
  UserContentBlock,
} from '@demi/core'
import type { AgentProvider, ToolDefinition } from '@demi/provider'
import type { Transcript } from './transcript'

export interface AgentPromptContext<State> {
  state: State
  cwd: string
  transcript: Transcript
}

export interface AgentToolContext<State> {
  state: State
  cwd: string
}

export interface AgentCommandContext<State> {
  state: State
  cwd: string
}

export interface AgentCommandSpec {
  name: string
  summary: string
}

export interface AgentDisposeContext<State> {
  state: State
  cwd: string
  transcript: Transcript
}

export interface AgentReferenceResolveContext<State> {
  state: State
  cwd: string
  transcript: Transcript
  signal: AbortSignal
}

export interface AgentToolInvokeContext<State> {
  state: State
  cwd: string
  toolCallId: string
  signal: AbortSignal
  emitProgress(progress: unknown): void
}

export interface AgentToolInvokeResult {
  output: ToolResultContentBlock[]
  isError?: boolean
  metadata?: unknown | null
  continuation?: ToolContinuation
}

export interface ToolContinuation {
  toolCallId: string
  sessionId: string
  status: 'running'
}

export interface AgentTool<State = unknown> extends ToolDefinition {
  invoke(ctx: AgentToolInvokeContext<State>, input: unknown): Promise<AgentToolInvokeResult> | AgentToolInvokeResult
}

export type AgentLifecycleEvent<State> =
  | {
      type: 'before_round_start'
      state: State
      transcript: Transcript
      content: UserContentBlock[]
    }
  | {
      type: 'after_tool_call'
      state: State
      transcript: Transcript
      toolCallId: string
      toolName: string
      result: AgentToolInvokeResult
    }
  | { type: 'after_transcript_rewrite'; state: State; transcript: Transcript; reason: 'retry' }

export interface AgentDefinition<State> {
  name: string
  initialState(): State
  systemPrompt(ctx: AgentPromptContext<State>): string
  preamble?(ctx: AgentPromptContext<State>): string | null
  resolveReferences?(
    ctx: AgentReferenceResolveContext<State>,
    content: UserContentBlock[],
  ): Promise<UserContentBlock[]> | UserContentBlock[]
  tools(ctx: AgentToolContext<State>): AgentTool<State>[]
  commands?(ctx: AgentCommandContext<State>): AgentCommandSpec[]
  lifecycle?(event: AgentLifecycleEvent<State>): Promise<void> | void
  dispose?(ctx: AgentDisposeContext<State>): Promise<void> | void
}

export interface AgentSessionParams<State> {
  provider: AgentProvider
  model: ModelSelection
  cwd: string
  definition: AgentDefinition<State>
  transcript?: CoreTranscript | Transcript
}

export interface AgentSessionSnapshot<State> {
  transcript: CoreTranscript
  state: State
  phase: SessionPhase
  queue: QueuedMessage[]
  cwd: string
  model: ModelSelection
  definitionName: string
}

export interface AgentSessionStore<State = unknown> {
  saveSnapshot(snapshot: AgentSessionSnapshot<State>): Promise<void> | void
}

export interface AgentSessionOptions<State = unknown> {
  idFactory?: () => string
  now?: () => string
  store?: AgentSessionStore<State>
  compaction?: {
    keepRecentTokens?: number
    preflightThresholdRatio?: number
  }
}

export type SessionEvent =
  | { type: 'transcript_changed'; transcript: CoreTranscript }
  | { type: 'phase_changed'; phase: SessionPhase }
  | { type: 'queue_changed'; queue: QueuedMessage[] }
  | { type: 'tool_progress'; toolCallId: string; toolName: string; progress: unknown }
  | { type: 'error'; error: Error }

export type SessionEventListener = (event: SessionEvent) => void

export interface ExternalMutationReservation {
  release(): void
}
