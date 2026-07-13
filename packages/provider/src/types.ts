import type {
  ModelSelection,
  ThinkingConfig,
  ThinkingEffort,
  TokenUsage,
  ToolResultContentBlock,
  UserContentBlock,
} from '@demicodes/core'
import type { ProviderQuota } from './quota'

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
  | { type: 'user_steer'; turnId: string; content: UserContentBlock[] }
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
  /**
   * End of a model response. `usage` MUST be the usage of a single API request
   * — the final one when the provider made several internally (e.g. a CLI turn
   * with tool calls). The agent anchors its context-size estimation on it
   * (input + output + cache reads/writes ≈ the context the next request will
   * carry), so a turn-cumulative total here inflates the estimate and triggers
   * spurious compaction.
   */
  | { type: 'response'; usage: TokenUsage }
  | {
      type: 'error'
      message: string
      code: string | null
      /** Server-suggested retry delay (e.g. from a Retry-After header), if any. */
      retryAfterMs?: number
    }
  | { type: 'abort' }

// ── provider ────────────────────────────────────────────────────────

export interface InferenceSteer {
  id: string
  sessionId: string
  turnId: string
  content: UserContentBlock[]
}

export interface ProviderRun extends AsyncIterable<ProviderEvent> {
  steer?(input: InferenceSteer): Promise<void> | void
}

export interface AgentProvider {
  run(request: InferenceRequest): ProviderRun
  /**
   * Releases any resources the provider holds open across turns — e.g. a long-lived CLI
   * subprocess kept alive for a whole session. Called once when the owning session closes.
   */
  dispose?(): Promise<void> | void
}

// ── public provider shell ───────────────────────────────────────────

export interface ProviderSelection {
  providerId: string
  model: ModelSelection
}

export type ProviderAuthState =
  | { status: 'unknown'; message?: string }
  | { status: 'authenticated'; accountLabel?: string }
  | { status: 'unauthenticated'; message?: string }
  | { status: 'error'; message: string }

export interface ProviderAuth {
  status(): Promise<ProviderAuthState> | ProviderAuthState
}

/** Public metadata only — never tokens, cookies, or raw auth files. */
export interface ProviderCredentialInfo {
  /** Stable id within this provider (not globally unique across providers). */
  id: string
  /** Human label: email, account id, or import tag. */
  label: string
  /** Optional secondary display (plan name, issuer, …). */
  detail?: string | null
  /** ISO-8601 when this entry was last imported or refreshed in the pool. */
  updatedAt?: string | null
}

export interface ProviderCredentialActive {
  credentialId: string | null
  /** Same shape as auth status, for the active credential. */
  status: ProviderAuthState
}

/**
 * User-facing material issued mid-flow by a device-code login: the product
 * relays it to the user, who completes login from any browser on any device.
 */
export interface ProviderCredentialLoginPending {
  /** URL the user opens to confirm the login. */
  verificationUrl: string
  /** One-time code the user enters at the verification URL (device-code flows). */
  userCode?: string | null
  /** ISO-8601 expiry of the code, when the vendor exposes one. */
  expiresAt?: string | null
  /**
   * True when the vendor displays a code AFTER approval that the user must
   * bring back; the product collects it via `promptForCode`.
   */
  requiresCodeInput?: boolean
}

export interface ProviderCredentialLoginOptions {
  /** Abort the login flow. */
  signal?: AbortSignal
  /** Fires once when the flow issues user-facing material (device-code login). */
  onPending?: (pending: ProviderCredentialLoginPending) => void
  /** Collects the code the user copied back from the vendor page (`requiresCodeInput` flows). */
  promptForCode?: () => Promise<string>
  /** Prefer browser vs device/CLI when the vendor supports both; best-effort. */
  preferBrowser?: boolean
}

/**
 * Login invoke result. Providers with a public device-code protocol (codex)
 * complete the flow natively, import the material into the pool, and return
 * the pool `credentialId`. CLI-spawning providers report vendor process status
 * only; the product should `importDefault` afterwards.
 */
export type ProviderCredentialLoginResult =
  | { status: 'completed'; credentialId?: string }
  | { status: 'cancelled' }
  | { status: 'unavailable'; message: string }
  | { status: 'failed'; message: string }

/**
 * Provider-specific add payloads. Concrete packages document accepted variants.
 * Do not put secrets on browser-visible control protocols.
 */
export type ProviderCredentialAddInput = {
  [key: string]: unknown
}

export type ProviderCredentialsCapability =
  | { mode: 'none' }
  | {
      mode: 'supported'
      /** Can spawn / open vendor login (`beginLogin`). */
      canBeginLogin?: boolean
      /** Can import from the vendor default location into the pool. */
      canImportDefault?: boolean
      /** Can register externally supplied material (`add`). */
      canAdd?: boolean
      /** Pool can hold more than one credential. */
      multi?: boolean
    }

/**
 * Multi-credential pool + process-global active switch.
 * See `docs/provider-global-credentials.md`.
 */
export interface ProviderCredentials {
  capability(): ProviderCredentialsCapability

  list(): Promise<ProviderCredentialInfo[]> | ProviderCredentialInfo[]

  getActive(): Promise<ProviderCredentialActive> | ProviderCredentialActive

  /**
   * Make `credentialId` the process-global active credential for this provider.
   * Subsequent auth / quota / inference use it.
   */
  setActive(credentialId: string): Promise<ProviderCredentialActive> | ProviderCredentialActive

  /**
   * Invoke the vendor’s own login flow. Does not complete OAuth inside demi
   * and does not return a credential id.
   */
  beginLogin?(options?: ProviderCredentialLoginOptions): Promise<ProviderCredentialLoginResult>

  /**
   * Snapshot current vendor-default material into the demi pool.
   * Assigns a stable `id` and returns public metadata (no secrets).
   */
  importDefault?(): Promise<ProviderCredentialInfo>

  /** Register material supplied by the product. */
  add?(input: ProviderCredentialAddInput): Promise<ProviderCredentialInfo>

  /** Remove a pool entry. */
  remove?(credentialId: string): Promise<void>
}

export type ProviderRuntimeState =
  | { status: 'unknown'; message?: string }
  | { status: 'ready'; message?: string }
  | { status: 'unavailable'; message: string }
  | { status: 'error'; message: string }

export interface Provider {
  id: string
  displayName: string
  auth?: ProviderAuth
  /** Optional subscription / rate-limit quota surface (`@demicodes/provider` quota helpers). */
  quota?: ProviderQuota
  /** Optional multi-credential pool + global active switch. */
  credentials?: ProviderCredentials
  state?(): Promise<ProviderRuntimeState> | ProviderRuntimeState
  listModels?(): Promise<ProviderModelList> | ProviderModelList
}

export interface ProviderRuntimeFactory {
  createRuntime(selection: ProviderSelection): Promise<AgentProvider> | AgentProvider
}

export interface ProviderFactoryDefinition extends Provider {
  createRuntime(selection: ProviderSelection): Promise<AgentProvider> | AgentProvider
}

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
  /** Whether the model accepts native video input (not frame extraction). Most models
   *  (all current Anthropic/Claude Code models) do not — their API has no video block.
   *  Optional: unset/undefined means "no video", so existing catalogs need no change. */
  supportsVideo?: boolean | null
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

export type ModelPolicy = {
  include?: string[]
  exclude?: string[]
  default?: string
}
