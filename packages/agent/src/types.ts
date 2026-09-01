import type {
  ModelSelection,
  ProviderErrorDiagnostics,
  QueuedMessage,
  SessionPhase,
  ToolResultContentBlock,
  Transcript as CoreTranscript,
  UserContentBlock,
} from '@demicodes/core'
import type { AgentProvider, ToolDefinition } from '@demicodes/provider'
import type { Command, Host } from '@demicodes/shell'
import type { PortableJsonValue } from '@demicodes/utils'
import type { TranscriptPatch } from './frames'
import type { TurnRetryPolicy } from './retry-policy'
import type { TranscriptLog } from './transcript'

/** Caller-defined data carried with one agent action. Demi transports it without interpreting it. */
export type AgentMetadata = Readonly<Record<string, PortableJsonValue>>

export interface AgentPromptContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  transcript: TranscriptLog
  metadata: AgentMetadata | null
}

export interface AgentSystemPromptContext<State> extends AgentPromptContext<State> {
  /**
   * Rendered help for every registered command (summary, subcommands, parameters,
   * stdin fields), produced by the server from the session's actual
   * CommandRegistry. Harnesses embed it wherever their system prompt wants the
   * command reference; empty string when no commands are registered.
   */
  commandsPrompt: string
}

export interface AgentToolContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  metadata: AgentMetadata | null
}

export interface AgentDisposeContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  transcript: TranscriptLog
}

export interface AgentReferenceResolveContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  transcript: TranscriptLog
  signal: AbortSignal
  metadata: AgentMetadata | null
}

export interface AgentHarnessContext<State> {
  state: State
  cwd: string
}

/** Context used when a shell operation resolves its action-specific Host. */
export interface AgentHostContext<State> extends AgentHarnessContext<State> {
  agentSessionId: string
  metadata: AgentMetadata | null
}

/**
 * A named subagent configuration. Every field is an override on the parent's
 * setup: omitted fields inherit the parent harness (system prompt), the
 * parent's registered commands, the parent Host, and the parent model.
 */
export interface SubagentProfile<State = unknown> {
  name: string
  /** Shown in the spawn command help so the parent model can pick a profile. */
  description: string
  systemPrompt?(ctx: AgentSystemPromptContext<State>): Promise<string> | string
  /** Derives the child's registered commands from the parent's list. */
  commands?(parentCommands: Command[]): Command[]
  /** Reject filesystem writes and process spawns on the child's Host. */
  readonly?: boolean
  /** When false, the child cannot spawn subagents of its own (communication and reads remain). */
  canSpawnSubagents?: boolean
  model?: ModelSelection
}

export interface AgentHarness<State = unknown> {
  name: string
  initialState(): State
  /** Return the same Host object for calls that target the same execution environment. */
  host(ctx: AgentHarnessContext<State> | AgentHostContext<State>): Host | Promise<Host>
  commands?(ctx: AgentHarnessContext<State>): Promise<Command[]> | Command[]
  /**
   * Named subagent profiles for `demi agent`. Omitted: one implicit profile
   * named `default` that fully inherits the parent's setup.
   */
  agents?(ctx: AgentHarnessContext<State>): Promise<SubagentProfile<State>[]> | SubagentProfile<State>[]
  systemPrompt(ctx: AgentSystemPromptContext<State>): Promise<string> | string
  preamble?(ctx: AgentPromptContext<State>): Promise<string | null> | string | null
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
  model: ModelSelection
  toolCallId: string
  signal: AbortSignal
  metadata: AgentMetadata | null
  emitProgress(progress: unknown): void
}

export interface AgentToolInvokeResult {
  output: ToolResultContentBlock[]
  isError?: boolean
  /** Bounded UI-facing view data stored on the tool_call block; see the core Block contract. */
  view?: unknown | null
  stopAfterToolResult?: boolean
}

/**
 * When a recorded model/provider switch is applied:
 * - 'next_turn' (default): the start of the next queued action — a running turn, if any,
 *   finishes entirely on the old model.
 * - 'immediate': the next inference boundary — mid-turn that is the next sampling/tool
 *   continuation, so the very next request already runs on the new model.
 */
export type ModelSwitchApply = 'immediate' | 'next_turn'

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
      transcript: TranscriptLog
      content: UserContentBlock[]
      metadata: AgentMetadata | null
    }
  | {
      type: 'after_tool_call'
      agentSessionId: string
      state: State
      transcript: TranscriptLog
      toolCallId: string
      toolName: string
      result: AgentToolInvokeResult
      metadata: AgentMetadata | null
    }
  | {
      type: 'after_transcript_rewrite'
      agentSessionId: string
      state: State
      transcript: TranscriptLog
      reason: 'retry'
      metadata: AgentMetadata | null
    }

export interface AgentHarnessRuntime<State> {
  harnessName: string
  initialState(): State
  systemPrompt(ctx: AgentPromptContext<State>): Promise<string> | string
  preamble?(ctx: AgentPromptContext<State>): Promise<string | null> | string | null
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
  transcript?: CoreTranscript | TranscriptLog
  state?: State
}

/**
 * Overrides for `AgentSession.clone()`.
 *
 * Omitted fields keep the clone defaults: an independent `provider.clone()`,
 * structured copies of model/transcript/state, and the same cwd/runtime.
 */
export interface AgentSessionCloneParams<State = unknown> {
  transcript?: CoreTranscript
  /** Use this runtime instead of `this.provider.clone()`. */
  provider?: AgentProvider
  model?: ModelSelection
  cwd?: string
  runtime?: AgentHarnessRuntime<State>
  /**
   * Replace state. When omitted, state is `structuredClone`d for isolation.
   * When provided, ownership transfers by reference (same as the constructor).
   */
  state?: State
  /** Options for the clone. Parent store is never inherited. */
  options?: AgentSessionOptions<State>
}

export interface AgentSessionCheckpoint<State> {
  transcript: CoreTranscript
  state: State
  phase: SessionPhase
  queue: QueuedMessage[]
  cwd: string
  model: ModelSelection
  harnessName: string
}

export interface AgentSessionStore<State = unknown> {
  saveCheckpoint(checkpoint: AgentSessionCheckpoint<State>): Promise<void> | void
  /** Load a previously saved checkpoint for this session, or null if none exists. */
  loadCheckpoint(): Promise<AgentSessionCheckpoint<State> | null>
}

export interface AgentSessionRestoreParams<State> {
  provider: AgentProvider
  runtime: AgentHarnessRuntime<State>
  checkpoint: AgentSessionCheckpoint<State>
}

export interface AgentSessionOptions<State = unknown> {
  agentSessionId?: string
  idFactory?: () => string
  now?: () => string
  store?: AgentSessionStore<State>
  compaction?: {
    keepRecentTokens?: number
    preflightThresholdRatio?: number
    /**
     * Absolute token threshold for auto-compaction. When set, replaces the
     * ratio-derived threshold (still clamped to the model context window).
     * A non-finite `preflightThresholdRatio` still disables auto-compaction.
     */
    preflightThresholdTokens?: number
  }
  /**
   * Maximum interval between checkpoint writes while a turn is streaming. Writes
   * always flush at action boundaries (turn end, abort, dispose); this only
   * bounds staleness during streaming. Default 1000ms.
   */
  persistIntervalMs?: number
  /** Overrides for the transient-failure retry policy (see TurnRetryPolicy). */
  retry?: Partial<TurnRetryPolicy>
}

export type SessionEvent =
  | { type: 'transcript_changed'; patches: TranscriptPatch[]; revision: number }
  | { type: 'phase_changed'; phase: SessionPhase }
  | { type: 'queue_changed'; queue: QueuedMessage[] }
  | { type: 'tool_progress'; toolCallId: string; toolName: string; progress: unknown }
  | {
      type: 'retry_scheduled'
      attempt: number
      delayMs: number
      code: string | null
      diagnostics?: ProviderErrorDiagnostics
    }
  | { type: 'error'; error: Error }

export type SessionEventListener = (event: SessionEvent) => void

export interface ExternalMutationReservation {
  release(): void
}
