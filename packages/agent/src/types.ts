import type {
  ModelSelection,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
  Transcript as CoreTranscript,
  UserContentBlock,
} from '@demi/core'
import type { AgentProvider, ToolDefinition } from '@demi/provider'
import type { CommandSpec, Host } from '@demi/shell'
import type { Transcript } from './transcript'

export interface AgentPromptContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  transcript: Transcript
}

export interface AgentToolContext<State> {
  agentSessionId: string
  state: State
  cwd: string
}

export interface AgentDisposeContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  transcript: Transcript
}

export interface AgentReferenceResolveContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  transcript: Transcript
  signal: AbortSignal
}

export interface AgentHarnessContext<State> {
  state: State
  cwd: string
}

export interface AgentHarness<State = unknown> {
  name: string
  initialState(): State
  host(ctx: AgentHarnessContext<State>): Host
  commands?(ctx: AgentHarnessContext<State>): CommandSpec[]
  systemPrompt(ctx: AgentPromptContext<State>): string
  preamble?(ctx: AgentPromptContext<State>): string | null
  resolveReferences?(
    ctx: AgentReferenceResolveContext<State>,
    content: UserContentBlock[],
  ): Promise<UserContentBlock[]> | UserContentBlock[]
  lifecycle?(event: AgentLifecycleEvent<State>): Promise<void> | void
  dispose?(ctx: AgentDisposeContext<State>): Promise<void> | void
}

export interface AgentToolInvokeContext<State> {
  agentSessionId: string
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
  stopAfterToolResult?: boolean
}

export interface ToolContinuation {
  toolCallId: string
  shellId: string
  commandId: string
  status: 'running'
}

export type AbortTarget =
  | 'active_provider_stream'
  | 'active_tool'
  | 'active_compaction'
  | 'active_turn'
  | 'queued_action'
  | 'queued_message'
  | 'pending_yield_wakeup'

export interface AbortResult {
  aborted: boolean
  target: AbortTarget | null
  canAbortAgain: boolean
}

export interface AgentTool<State = unknown> extends ToolDefinition {
  invoke(ctx: AgentToolInvokeContext<State>, input: unknown): Promise<AgentToolInvokeResult> | AgentToolInvokeResult
}

export type AgentLifecycleEvent<State> =
  | {
      type: 'before_round_start'
      agentSessionId: string
      state: State
      transcript: Transcript
      content: UserContentBlock[]
    }
  | {
      type: 'after_tool_call'
      agentSessionId: string
      state: State
      transcript: Transcript
      toolCallId: string
      toolName: string
      result: AgentToolInvokeResult
    }
  | { type: 'after_transcript_rewrite'; agentSessionId: string; state: State; transcript: Transcript; reason: 'retry' }

export interface AgentHarnessRuntime<State> {
  harnessName: string
  initialState(): State
  systemPrompt(ctx: AgentPromptContext<State>): string
  preamble?(ctx: AgentPromptContext<State>): string | null
  resolveReferences?(
    ctx: AgentReferenceResolveContext<State>,
    content: UserContentBlock[],
  ): Promise<UserContentBlock[]> | UserContentBlock[]
  tools(ctx: AgentToolContext<State>): AgentTool<State>[]
  lifecycle?(event: AgentLifecycleEvent<State>): Promise<void> | void
  dispose?(ctx: AgentDisposeContext<State>): Promise<void> | void
}

export interface AgentSessionParams<State> {
  provider: AgentProvider
  model: ModelSelection
  cwd: string
  runtime: AgentHarnessRuntime<State>
  transcript?: CoreTranscript | Transcript
  state?: State
}

export interface AgentSessionSnapshot<State> {
  transcript: CoreTranscript
  state: State
  phase: SessionPhase
  queue: QueuedMessage[]
  cwd: string
  model: ModelSelection
  harnessName: string
}

export interface AgentSessionStore<State = unknown> {
  saveSnapshot(snapshot: AgentSessionSnapshot<State>): Promise<void> | void
}

export interface AgentSessionRestoreParams<State> {
  provider: AgentProvider
  runtime: AgentHarnessRuntime<State>
  snapshot: AgentSessionSnapshot<State>
}

export interface AgentSessionOptions<State = unknown> {
  agentSessionId?: string
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
