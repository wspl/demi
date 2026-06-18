import type {
  ThinkingConfig,
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
  modelId: string
  systemPrompt: string
  cwd: string
  items: InferenceItem[]
  tools: ToolDefinition[]
  thinking: ThinkingConfig | null
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

export interface ProviderDefinition<Config = unknown> {
  type: string
  displayName: string
  auth?: ProviderAuth
  state?(): Promise<ProviderRuntimeState> | ProviderRuntimeState
  createProvider(config: Config): Promise<AgentProvider> | AgentProvider
}
