import type {
  Block,
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
import type { TranscriptPatch } from './protocol/frames'
import type { TurnRetryPolicy } from './session/retry-policy'
import type { TranscriptLog } from './transcript/transcript'

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

/**
 * Context used when a shell operation resolves its action-specific Host.
 * `agentSessionId` is the root session's: the execution target belongs to
 * the conversation, and a subagent resolves through its root.
 */
export interface AgentHostContext<State> extends AgentHarnessContext<State> {
  agentSessionId: string
  metadata: AgentMetadata | null
}

/** Context for building the session's registered commands (root session id). */
export interface AgentCommandsContext<State> extends AgentHarnessContext<State> {
  agentSessionId: string
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
  /** When false, the child cannot spawn subagents of its own (communication and reads remain). */
  canSpawnSubagents?: boolean
  model?: ModelSelection
}

export interface AgentHarness<State = unknown> {
  name: string
  initialState(): State
  /** Return the same Host object for calls that target the same execution environment. */
  host(ctx: AgentHarnessContext<State> | AgentHostContext<State>): Host | Promise<Host>
  commands?(ctx: AgentCommandsContext<State>): Promise<Command[]> | Command[]
  /**
   * Named subagent profiles for `demi agent`. Omitting --profile inherits
   * the parent's setup; the name `default` is reserved.
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

/** Everything a session persists except the transcript blocks. */
export interface AgentSessionStateSnapshot<State> {
  state: State
  phase: SessionPhase
  queue: QueuedMessage[]
  cwd: string
  model: ModelSelection
  harnessName: string
}

/**
 * One persist tick: the journal write. `changedBlocks` carries only the
 * transcript blocks mutated since the last save (streaming appends touch the
 * tail; compaction and rewinds touch a suffix), keyed by transcript index.
 * Rows at `index >= blockCount` no longer exist and must be deleted. The
 * state snapshot rides along on every tick — it is small.
 */
export interface AgentSessionPersistUpdate<State> extends AgentSessionStateSnapshot<State> {
  changedBlocks: Array<{ index: number; block: Block }>
  blockCount: number
}

/**
 * One node's persistence: block rows plus a state row. Implementations write
 * `changedBlocks` as individual rows — never the whole transcript — and
 * reassemble the checkpoint on load. A save is one commit.
 */
export interface AgentSessionStore<State = unknown> {
  save(update: AgentSessionPersistUpdate<State>): Promise<void> | void
  /** Load the persisted session, or null if none exists. */
  load(): Promise<AgentSessionCheckpoint<State> | null>
}

/** The phase a node closed in. */
export type AgentNodeClosePhase = 'completed' | 'aborted' | 'error'

/**
 * A node of the session tree as the store holds it: identity and
 * relationship, never runtime state (`docs/subagent.md` § Persistence). The
 * root has no parent; a node is archived once `closedPhase` is set.
 */
export interface AgentNodeRecord {
  id: string
  parentId: string | null
  /** Short title; empty for the root. */
  description: string
  /** The profile the node was spawned with; null inherits the parent's setup (and for the root). */
  profileName: string | null
  /** Action metadata of the round that spawned it — Host routing; null for the root. */
  metadata: AgentMetadata | null
  spawnedAt: number
  canSpawnSubagents: boolean
  /** null while live; the phase it closed in once archived. */
  closedPhase: AgentNodeClosePhase | null
  closedAt: number | null
  /** The bounded last assistant text of a completed close; null otherwise. */
  result: string | null
  /** The reason of an error close; null otherwise. */
  failure: string | null
  /** Whether the completion has reached its return path (`docs/subagent.md` § Persistence). */
  delivered: boolean
}

/** What a close writes, in one commit after the node's final checkpoint. */
export interface AgentNodeClose {
  phase: AgentNodeClosePhase
  closedAt: number
  result: string | null
  failure: string | null
}

/**
 * The session tree's persistence, one store per root: node rows with their
 * checkpoints. Create, a node's save and close are each one commit whatever
 * the realization (`docs/subagent.md` § Persistence).
 */
export interface AgentTreeStore<State = unknown> {
  node(id: string): Promise<AgentNodeRecord | null>
  /** Direct children in spawn order, live and archived alike. */
  children(parentId: string): Promise<AgentNodeRecord[]>
  /** The node row and its initial checkpoint — the first message queued in it — as one commit. */
  createNode(record: AgentNodeRecord, checkpoint: AgentSessionPersistUpdate<State>): Promise<void>
  /**
   * The node's journal. A save is one commit that also marks delivered every
   * child completion the saved state carries (`completedChildrenCarriedBy`).
   */
  sessionStore(id: string): AgentSessionStore<State>
  /** The close row, one commit after the final checkpoint; the completion starts undelivered. */
  closeNode(id: string, close: AgentNodeClose): Promise<void>
  /** An archived node live again — this round's metadata, a fresh spawn time, the reviving message queued — as one commit. */
  reopenNode(id: string, fields: { metadata: AgentMetadata | null; spawnedAt: number }, message: QueuedMessage): Promise<void>
  /** The completion reached its parent by a path the parent's checkpoint cannot show. */
  markDelivered(id: string): Promise<void>
  /** The node and every descendant with all their rows. */
  deleteNode(id: string): Promise<void>
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
