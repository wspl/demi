// The wire boundary in both directions: client frames and server frames are
// declared here as zod schemas and `frames.ts` derives `ClientFrame` and
// `ServerFrame` from them — one declaration per direction, validated at
// AgentServer transport ingress and at AgentClient frame ingress. Shared types
// owned by other packages (core content blocks, provider selections) keep their
// hand-written types as the source of truth; their validators carry a
// `z.ZodType<T>` annotation so schema drift is a compile error, not a runtime
// surprise.
import { z } from 'zod'
import type {
  Block,
  DocumentSource,
  ImageSource,
  Model,
  ModelSelection,
  PendingSteer,
  ProviderErrorDiagnostics,
  QueuedMessage,
  SessionPhase,
  ThinkingCapability,
  ThinkingConfig,
  TokenUsage,
  ToolResultContentBlock,
  UserContentBlock,
  VideoSource,
} from '@demicodes/core'
import type { FileExtension } from '@demicodes/core'
import type { ProviderSelection } from '@demicodes/provider'
import type { ShellCommandStatus } from '@demicodes/shell'
import type { PortableJsonValue } from '@demicodes/utils'
import type { AbortResult, AgentMetadata, ModelSwitchApply } from '../types'
import type { PendingInternalSteer } from '../session/steer-queue'
import { agentMessageSchema } from './agent-message'

// ── shared-type validators ──────────────────────────────────────────

// The extension list is a closed core-owned set; the wire validates
// stringness and leaves membership to the catalog code that consumes it.
const fileExtensionSchema = z.custom<FileExtension>((value) => typeof value === 'string')

const thinkingCapabilitySchema: z.ZodType<ThinkingCapability> = z.discriminatedUnion(
  'type',
  [
    z.object({
      type: z.literal('adaptive'),
      efforts: z.array(z.string()),
      defaultEffort: z.string().nullable()
    }),
    z.object({
      type: z.literal('budget'),
      minBudgetTokens: z.number().nullable(),
      maxBudgetTokens: z.number().nullable(),
      defaultBudgetTokens: z.number().nullable(),
    }),
    z.object({
      type: z.literal('effort'),
      efforts: z.array(z.string()),
      defaultEffort: z.string().nullable(),
      summaries: z.array(z.enum(['auto', 'concise', 'detailed', 'off', 'on'])),
      defaultSummary: z.enum(['auto', 'concise', 'detailed', 'off', 'on'])
        .nullable(),
    }),
    z.object({ type: z.literal('disabled') }),
  ]
)

export const thinkingConfigSchema: z.ZodType<ThinkingConfig> = z.discriminatedUnion(
  'type',
  [
    z.object({ type: z.literal('adaptive'), effort: z.string() }),
    z.object({ type: z.literal('budget'), budgetTokens: z.number() }),
    z.object({
      type: z.literal('effort'),
      effort: z.string(),
      summary: z.enum(['auto', 'concise', 'detailed', 'off', 'on']).nullable(),
    }),
    z.object({ type: z.literal('disabled') }),
  ]
)

const modelSchema: z.ZodType<Model> = z.object({
  id: z.string(),
  name: z.string(),
  contextWindow: z.number(),
  outputLimit: z.number().int().positive().nullable(),
  inputLimit: z.number().nullable(),
  thinking: z.array(thinkingCapabilitySchema),
  acceptedExtensions: z.array(fileExtensionSchema).nullable(),
})

export const modelSelectionSchema: z.ZodType<ModelSelection> = z.object({
  providerId: z.string(),
  model: modelSchema,
  thinking: thinkingConfigSchema.nullable(),
  serviceTierId: z.string().nullable().optional(),
})

const providerSelectionSchema: z.ZodType<ProviderSelection> = z.object({
  providerId: z.string(),
  model: modelSelectionSchema,
})

const imageSourceSchema: z.ZodType<ImageSource> = z.union([
  z.object({
    type: z.literal('binary'),
    data: z.instanceof(Uint8Array),
    mediaType: z.string()
  }),
  z.object({ type: z.literal('url'), url: z.string() }),
])

const videoSourceSchema: z.ZodType<VideoSource> = z.union([
  z.object({
    type: z.literal('binary'),
    data: z.instanceof(Uint8Array),
    mediaType: z.string()
  }),
  z.object({ type: z.literal('url'), url: z.string() }),
])

const documentSourceSchema: z.ZodType<DocumentSource> = z.object({
  data: z.instanceof(Uint8Array),
  mediaType: z.string(),
  fileName: z.string(),
})

export const userContentBlockSchema: z.ZodType<UserContentBlock> = z.discriminatedUnion(
  'type',
  [
    z.object({ type: z.literal('text'), text: z.string() }),
    z.object({ type: z.literal('image'), source: imageSourceSchema }),
    z.object({ type: z.literal('video'), source: videoSourceSchema }),
    z.object({ type: z.literal('document'), source: documentSourceSchema }),
    z.object({ type: z.literal('reference'), reference: z.string() }),
    z.object({
      type: z.literal('attachment'),
      name: z.string().min(1),
      path: z.string().min(1),
      mediaType: z.string(),
      sizeBytes: z.number().int().nonnegative(),
      sha256: z.string().min(1),
      snippet: z.string().optional(),
    }),
  ]
)

export const transcriptVersionSchema = z.object({
  epoch: z.string().min(1),
  revision: z.number().int().nonnegative(),
})

export const editRequestSchema = z.object({
  operationId: z.string().min(1),
  targetBlockId: z.string().min(1),
  version: transcriptVersionSchema,
  content: z.array(userContentBlockSchema).min(1).refine(
    (content) => content.some((part) => part.type !== 'text' || part.text.trim()),
    'A message must contain text or an attachment',
  ),
})

export const editReceiptSchema = z.object({
  operationId: z.string().min(1),
  digest: z.string().regex(/^[a-f0-9]{64}$/),
  turnId: z.string().min(1),
})

export const editResultSchema = z.discriminatedUnion('status', [
  z.object({
    type: z.literal('edit_result'),
    operationId: z.string().min(1),
    status: z.literal('accepted'),
    turnId: z.string().min(1),
  }),
  z.object({
    type: z.literal('edit_result'),
    operationId: z.string().min(1),
    status: z.literal('rejected'),
    reason: z.string(),
  }),
])

export type TranscriptVersion = z.infer<typeof transcriptVersionSchema>
export type EditRequest = z.infer<typeof editRequestSchema>
export type EditReceipt = z.infer<typeof editReceiptSchema>

// Accepted user steers returned by AgentServer; validated when AgentClient receives them.
const pendingSteerSchema: z.ZodType<PendingSteer> = z.object({
  id: z.string(),
  turnId: z.string(),
  model: modelSelectionSchema,
  content: z.array(userContentBlockSchema),
})

export const pendingSteersFrameSchema = z.object({
  type: z.literal('pending_steers'),
  pendingSteers: z.array(pendingSteerSchema),
})

export const portableJsonValueSchema: z.ZodType<PortableJsonValue> = z.lazy(() =>
  z.union([
    z.null(),
    z.boolean(),
    z.number(),
    z.string(),
    z.bigint(),
    z.instanceof(Uint8Array),
    z.instanceof(Date),
    z.array(portableJsonValueSchema),
    z.record(z.string(), portableJsonValueSchema),
  ]),
)

export const metadataSchema: z.ZodType<AgentMetadata> = z.record(
  z.string(),
  portableJsonValueSchema
)

// An agent message accepted but not yet materialized into the transcript, as a
// checkpoint carries it.
export const pendingInternalSteerSchema: z.ZodType<PendingInternalSteer> = z.object({
  turnId: z.string().min(1),
  model: modelSelectionSchema,
  agentMessage: agentMessageSchema,
  metadata: metadataSchema.nullable(),
}).strict()

const modelSwitchApplySchema: z.ZodType<ModelSwitchApply> = z.enum([
  'immediate',
  'next_turn'
])

// ── the client frames (single source of truth for ClientFrame) ──────

export const sendFrameSchema = z.object({
  type: z.literal('send'),
  messageId: z.string(),
  content: z.array(userContentBlockSchema),
  metadata: metadataSchema.optional(),
})

export const steerFrameSchema = z.object({
  type: z.literal('steer'),
  steerId: z.string(),
  content: z.array(userContentBlockSchema)
})

export const clientFrameSchema = z.discriminatedUnion('type', [
  z.object({
    type: z.literal('open'),
    provider: providerSelectionSchema,
    cwd: z.string(),
    sessionId: z.string()
  }),
  sendFrameSchema,
  z.object({
    type: z.literal('edit_and_send'),
    request: editRequestSchema,
    metadata: metadataSchema.optional(),
  }),
  z.object({ type: z.literal('dequeue_message'), messageId: z.string() }),
  z.object({ type: z.literal('send_queued_message'), messageId: z.string() }),
  z.object({
    type: z.literal('steer_queued_message'),
    messageId: z.string(),
    steerId: z.string()
  }),
  z.object({ type: z.literal('clear_message_queue') }),
  steerFrameSchema,
  z.object({ type: z.literal('cancel_pending_steer'), steerId: z.string() }),
  z.object({
    type: z.literal('set_provider'),
    provider: providerSelectionSchema,
    apply: modelSwitchApplySchema.optional()
  }),
  z.object({ type: z.literal('abort') }),
  z.object({ type: z.literal('abort_subagents') }),
  // One live child, with its subtree; it settles through its own `subagent closed` frame.
  z.object({ type: z.literal('abort_subagent'), subagentId: z.string() }),
  z.object({ type: z.literal('retry'), metadata: metadataSchema.optional() }),
  z.object({ type: z.literal('resume'), metadata: metadataSchema.optional() }),
  z.object({ type: z.literal('compact'), metadata: metadataSchema.optional() }),
  z.object({
    type: z.literal('shell_write'),
    commandId: z.string(),
    stdin: z.string(),
    metadata: metadataSchema.optional(),
  }),
  // Stops one running command; its final status arrives as a `shell_output` frame.
  z.object({
    type: z.literal('shell_abort'),
    commandId: z.string(),
    metadata: metadataSchema.optional(),
  }),
  // Requests a fresh transcript_reset; sent by the client when it detects a
  // revision gap in the patch stream (defensive resync, transports are ordered).
  z.object({ type: z.literal('sync_transcript') }),
  z.object({ type: z.literal('close') }),
])

// ── transcript blocks on the wire ───────────────────────────────────
//
// A block's media travels in one of two forms: the inline bytes a session
// produces, or the blob reference the backend rewrites them into for the page
// (`externalizeBlockMedia`). Both are accepted here. The decoded block keeps
// the core `Block` type, which is the view contract every renderer reads; a
// block still in the reference form is for storage and display only, and
// `rehydrateBlockMedia` restores the bytes before anything replays it to a
// provider.

/** A media source whose bytes were moved into the blob store. */
export const refSourceSchema = z.object({
  type: z.literal('ref'),
  ref: z.string().min(1),
  mediaType: z.string(),
  fileName: z.string().optional(),
})
export type RefSource = z.infer<typeof refSourceSchema>

/** The same for a tool result's base64 source, which carries no type tag. */
export const refBase64SourceSchema = z.object({
  ref: z.string().min(1),
  mediaType: z.string(),
})
export type RefBase64Source = z.infer<typeof refBase64SourceSchema>

const externalizedUserContentSchema = z.object({
  type: z.enum(['image', 'video', 'document']),
  source: refSourceSchema,
})

const wireUserContentSchema = z.union([
  userContentBlockSchema,
  externalizedUserContentSchema,
])

export const toolResultContentBlockSchema: z.ZodType<ToolResultContentBlock> = z
  .discriminatedUnion('type', [
    z.object({ type: z.literal('text'), text: z.string() }),
    z.object({
      type: z.literal('image'),
      source: z.object({ mediaType: z.string(), data: z.string() }),
    }),
    z.object({
      type: z.literal('video'),
      source: z.object({ mediaType: z.string(), data: z.string() }),
    }),
  ])

const externalizedToolResultSchema = z.object({
  type: z.enum(['image', 'video']),
  source: refBase64SourceSchema,
})

const wireToolResultSchema = z.union([
  toolResultContentBlockSchema,
  externalizedToolResultSchema,
])

const tokenUsageSchema: z.ZodType<TokenUsage> = z.object({
  inputTokens: z.number(),
  outputTokens: z.number(),
  cacheReadTokens: z.number(),
  cacheWriteTokens: z.number(),
})

const providerErrorDiagnosticsSchema: z.ZodType<ProviderErrorDiagnostics> = z
  .object({
    source: z.enum(['http', 'stream', 'transport', 'unknown']),
    clientRequestId: z.string().optional(),
    providerRequestId: z.string().optional(),
    providerResponseId: z.string().optional(),
    providerCode: z.string().optional(),
    httpStatus: z.number().optional(),
  })

const blockMeta = {
  id: z.string().min(1),
  createdAt: z.string(),
  model: modelSelectionSchema,
}

const wireBlockSchema = z.discriminatedUnion('type', [
  z.object({
    ...blockMeta,
    type: z.literal('agent_message'),
    turnId: z.string().min(1),
    message: agentMessageSchema,
  }).refine(
    (block) => block.id === block.message.id,
    'Receipt id must match its message',
  ),
  z.object({
    ...blockMeta,
    type: z.literal('user'),
    turnId: z.string(),
    content: z.array(wireUserContentSchema),
    preamble: z.string().nullable(),
    resolvedContent: z.array(wireUserContentSchema).optional(),
    hidden: z.boolean().optional(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('resume'),
    turnId: z.string(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('steer'),
    turnId: z.string(),
    content: z.array(wireUserContentSchema),
    hidden: z.boolean().optional(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('thinking'),
    text: z.string(),
    signature: z.string().nullable(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('redacted_thinking'),
    data: z.string(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('text'),
    text: z.string(),
    forkable: z.literal(true).optional(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('tool_call'),
    toolUseId: z.string(),
    toolName: z.string(),
    input: z.string(),
    status: z.enum(['executing', 'completed', 'error']),
    streamingOutput: z.array(wireToolResultSchema),
    output: z.array(wireToolResultSchema),
    view: z.unknown(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('response'),
    usage: tokenUsageSchema,
  }),
  z.object({
    ...blockMeta,
    type: z.literal('error'),
    message: z.string(),
    code: z.string().nullable(),
    diagnostics: providerErrorDiagnosticsSchema.optional(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('abort'),
    isResumed: z.boolean(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('compaction_boundary'),
    summary: z.string(),
    summaryTokens: z.number(),
  }),
  z.object({
    ...blockMeta,
    type: z.literal('compaction_marker'),
    boundaryId: z.string(),
    compactedTokens: z.number(),
  }),
  z.object({
    type: z.literal('extension_state_snapshot'),
    id: z.string().min(1),
    createdAt: z.string(),
    extensionName: z.string(),
    state: z.unknown(),
  }),
])

export const blockSchema: z.ZodType<Block> = wireBlockSchema.transform(
  // The reference form is wider than `Block`; see the note above.
  (block) => block as unknown as Block,
)

const blockPathSchema = z.tuple([
  z.literal('blocks'),
  z.number().int().nonnegative(),
])

export const transcriptPatchSchema = z.discriminatedUnion('op', [
  z.object({
    op: z.literal('add'),
    path: blockPathSchema,
    value: blockSchema
  }),
  z.object({ op: z.literal('remove'), path: blockPathSchema }),
  z.object({
    op: z.literal('replace_block'),
    path: blockPathSchema,
    value: blockSchema
  }),
  z.object({
    op: z.literal('append_text'),
    path: blockPathSchema,
    delta: z.string()
  }),
  z.object({
    op: z.literal('replace'),
    path: z.tuple([z.literal('blocks')]),
    value: z.array(blockSchema),
  }),
])

// ── the server frames (single source of truth for ServerFrame) ──────

const sessionPhaseSchema: z.ZodType<SessionPhase> = z.enum([
  'idle',
  'running',
  'compacting',
])

const queuedMessageSchema: z.ZodType<QueuedMessage> = z.object({
  id: z.string(),
  text: z.string(),
  content: z.array(userContentBlockSchema),
})

const abortResultSchema: z.ZodType<AbortResult> = z.object({
  aborted: z.boolean(),
  target: z.enum([
    'active_provider_stream',
    'active_tool',
    'active_compaction',
    'active_turn',
    'queued_action',
    'queued_message',
    'pending_yield_wakeup',
  ]).nullable(),
  canAbortAgain: z.boolean(),
})

const shellStreamViewSchema = z.object({
  path: z.string().optional(),
  offset: z.number(),
  delta: z.string(),
  tail: z.string(),
  bytes: z.number(),
  truncated: z.boolean(),
})

const shellOutputViewSchema = z.object({
  path: z.string().optional(),
  offset: z.number(),
  text: z.string(),
  tail: z.string(),
  chunks: z.array(z.object({
    stream: z.enum(['stdout', 'stderr']),
    text: z.string(),
  })),
  bytes: z.number(),
  truncated: z.boolean(),
})

const shellCommandMeta = {
  shellId: z.string(),
  commandId: z.string(),
  outputDir: z.string().optional(),
  stdout: shellStreamViewSchema,
  stderr: shellStreamViewSchema,
  output: shellOutputViewSchema,
  runningMs: z.number(),
  idleMs: z.number(),
}

export const shellCommandStatusSchema: z.ZodType<ShellCommandStatus> = z
  .discriminatedUnion('status', [
    z.object({
      ...shellCommandMeta,
      status: z.literal('running'),
      runningHint: z.string().optional(),
    }),
    z.object({ ...shellCommandMeta, status: z.literal('aborted') }),
    z.object({
      ...shellCommandMeta,
      status: z.literal('exited'),
      exitCode: z.number(),
      binaryStdout: z.object({
        data: z.instanceof(Uint8Array),
        truncated: z.boolean(),
        totalBytes: z.number(),
        limitBytes: z.number(),
      }).optional(),
    }),
  ])

/** One child agent session as seen on the parent connection. */
export const subagentJobSchema = z.object({
  subagentId: z.string(),
  parentSessionId: z.string(),
  description: z.string(),
  profile: z.string().nullable(),
  phase: z.enum(['running', 'completed', 'aborted', 'error']),
  startedAt: z.string(),
  endedAt: z.string().nullable(),
  /** Action metadata of the round that spawned the child. */
  metadata: metadataSchema.nullable(),
  /** Present on `closed`: the child's last assistant text, at most 32 KiB. */
  result: z.string().optional(),
})

const steerResultFrameSchema = z.discriminatedUnion('status', [
  z.object({
    type: z.literal('steer_result'),
    steerId: z.string(),
    status: z.literal('accepted'),
  }),
  z.object({
    type: z.literal('steer_result'),
    steerId: z.string(),
    status: z.literal('rejected'),
    reason: z.string(),
  }),
])

export const serverFrameSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('opened') }),
  editResultSchema,
  z.object({
    type: z.literal('rejected'),
    command: z.string(),
    reason: z.string()
  }),
  z.object({
    type: z.literal('transcript_reset'),
    blocks: z.array(blockSchema),
    ...transcriptVersionSchema.shape,
  }),
  z.object({
    type: z.literal('transcript_patch'),
    patches: z.array(transcriptPatchSchema),
    revision: z.number().int().nonnegative(),
  }),
  z.object({ type: z.literal('phase'), phase: sessionPhaseSchema }),
  z.object({ type: z.literal('queue'), queue: z.array(queuedMessageSchema) }),
  pendingSteersFrameSchema,
  steerResultFrameSchema,
  z.object({ type: z.literal('abort_result'), result: abortResultSchema }),
  z.object({
    type: z.literal('tool_progress'),
    toolUseId: z.string(),
    output: z.array(toolResultContentBlockSchema),
  }),
  z.object({
    type: z.literal('shell_output'),
    shellId: z.string(),
    commandId: z.string(),
    status: shellCommandStatusSchema,
  }),
  z.object({
    type: z.literal('shell_write_result'),
    commandId: z.string(),
    output: z.array(toolResultContentBlockSchema),
  }),
  // A transient provider failure is being retried with backoff; informational.
  z.object({
    type: z.literal('retry_scheduled'),
    attempt: z.number(),
    delayMs: z.number(),
    code: z.string().nullable(),
    diagnostics: providerErrorDiagnosticsSchema.optional(),
  }),
  z.object({
    type: z.literal('error'),
    message: z.string(),
    code: z.string().optional(),
    diagnostics: providerErrorDiagnosticsSchema.optional(),
  }),
  z.object({
    type: z.literal('subagent'),
    event: z.enum(['started', 'closed']),
    job: subagentJobSchema,
  }),
  z.object({
    type: z.literal('subagent_transcript_reset'),
    subagentId: z.string(),
    blocks: z.array(blockSchema),
    revision: z.number().int().nonnegative(),
  }),
  z.object({
    type: z.literal('subagent_transcript_patch'),
    subagentId: z.string(),
    patches: z.array(transcriptPatchSchema),
    revision: z.number().int().nonnegative(),
  }),
  z.object({ type: z.literal('closed') }),
])
