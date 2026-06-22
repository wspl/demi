import type {
  ThinkingConfig,
  ThinkingEffort,
  TokenUsage,
  ToolResultContentBlock,
  UserContentBlock,
} from '@demi/core'

// ── tool definition ─────────────────────────────────────────────────

export interface ToolDefinition {
  name: string
  description: string
  /** JSON Schema describing the tool input. */
  inputSchema: Record<string, unknown>
}

// ── inference item ──────────────────────────────────────────────────

export type InferenceItem =
  | { type: 'user_message'; content: UserContentBlock[] }
  | { type: 'assistant_text'; modelId: string; text: string }
  | {
      type: 'assistant_thinking'
      modelId: string
      text: string
      signature: string | null
    }
  | { type: 'assistant_redacted_thinking'; modelId: string; data: string }
  | {
      type: 'tool_use'
      modelId: string
      toolUseId: string
      toolName: string
      input: unknown
    }
  | {
      type: 'tool_result'
      toolUseId: string
      output: ToolResultContentBlock[]
      isError: boolean
    }

// ── inference request ───────────────────────────────────────────────

export interface InferenceRequest {
  /** Stable id for the owning agent session. */
  sessionId: string
  /** Stable id for the active user/maintenance turn; shared by provider continuations inside that turn. */
  turnId: string
  /** Unique id for this concrete provider request. */
  requestId: string
  modelId: string
  systemPrompt: string
  cwd: string
  items: InferenceItem[]
  tools: ToolDefinition[]
  thinking: ThinkingConfig | null
  serviceTierId?: string | null
  cancel: AbortSignal
}

// ── provider event ──────────────────────────────────────────────────

export type ProviderEvent =
  | { type: 'thinking_start' }
  | { type: 'thinking_delta'; text: string }
  | { type: 'thinking_signature'; signature: string }
  | { type: 'redacted_thinking'; data: string }
  | { type: 'text_delta'; text: string }
  | {
      type: 'tool_call_requested'
      toolUseId: string
      toolName: string
      input: unknown
    }
  | { type: 'response'; usage: TokenUsage }
  | { type: 'error'; message: string; code: string | null }
  | { type: 'abort' }

// ── provider ────────────────────────────────────────────────────────

export interface AgentProvider {
  run(request: InferenceRequest): AsyncIterable<ProviderEvent>
  /**
   * Releases any resources the provider holds open across turns — e.g. a long-lived CLI
   * subprocess kept alive for a whole session. Called once when the owning session closes.
   */
  dispose?(): Promise<void> | void
}

// ── provider registry / auth shell ──────────────────────────────────

export type ProviderAuthState =
  | { status: 'unknown'; message?: string }
  | { status: 'authenticated'; accountLabel?: string }
  | { status: 'unauthenticated'; message?: string }
  | { status: 'error'; message: string }

export interface ProviderAuth {
  status(): Promise<ProviderAuthState> | ProviderAuthState
}

export type ProviderRuntimeState =
  | { status: 'unknown'; message?: string }
  | { status: 'ready'; message?: string }
  | { status: 'unavailable'; message: string }
  | { status: 'error'; message: string }

// Provider model catalog.

export interface ProviderModelCost {
  input: number | null
  output: number | null
  cacheRead: number | null
  cacheWrite: number | null
}

export interface ProviderServiceTier {
  id: string
  label: string
  description?: string
}

export interface ProviderModel {
  providerId: string
  id: string
  displayName: string
  description?: string
  contextWindow: number | null
  outputLimit: number | null
  supportsTools: boolean | null
  supportsAttachments: boolean | null
  supportsReasoning: boolean | null
  supportedThinkingEfforts: ThinkingEffort[] | null
  defaultThinkingEffort: ThinkingEffort | null
  /** Whether thinking can be turned off entirely. Some transports (e.g. the Claude Code CLI, whose
   *  `--effort` flag only accepts low|medium|high|xhigh|max) can level thinking but never disable it,
   *  so the UI must not offer a "no reasoning" option. Defaults to true (optional) when unset. */
  canDisableThinking?: boolean | null
  serviceTiers?: ProviderServiceTier[] | null
  defaultServiceTierId?: string | null
  cost?: ProviderModelCost
  sourceFetchedAt: string
  stale: boolean
}

export interface ProviderModelList {
  providerId: string
  models: ProviderModel[]
  defaultModelId: string | null
  warnings: string[]
  sourceFetchedAt: string
  stale: boolean
}

export interface ProviderDefinition<Config = unknown> {
  type: string
  displayName: string
  auth?: ProviderAuth
  state?(): Promise<ProviderRuntimeState> | ProviderRuntimeState
  listModels?(config: Config): Promise<ProviderModelList> | ProviderModelList
  createProvider(config: Config): Promise<AgentProvider> | AgentProvider
}
