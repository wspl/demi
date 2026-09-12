// The inbound wire boundary: client frames are declared here as zod schemas
// and `frames.ts` derives the `ClientFrame` type from them — one declaration,
// validated at AgentServer transport ingress. Shared types owned by other
// packages (core content blocks, provider selections) keep their hand-written
// types as the source of truth; their validators carry a `z.ZodType<T>`
// annotation for static assignability; negative fixtures verify runtime constraints.
import { z } from 'zod'
import type {
  Model,
  ModelSelection,
  PendingSteer,
  ThinkingCapability,
  ThinkingConfig,
} from '@demicodes/core'
import { FILE_EXTENSIONS } from '@demicodes/core'
import type { ProviderSelection } from '@demicodes/provider'
import { base64ToBytes, type PortableJsonValue } from '@demicodes/utils'
import type { AgentMetadata, ModelSwitchApply } from '../types'

// ── shared-type validators ──────────────────────────────────────────

const fileExtensionSchema = z.enum(FILE_EXTENSIONS)

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
  contextWindow: z.number().int().positive(),
  outputLimit: z.number().int().positive().nullable(),
  inputLimit: z.number().int().positive().nullable(),
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

export const bytesSchema: z.ZodType<Uint8Array> = z.instanceof(Uint8Array)

export const binarySourceSchema = z.strictObject({
  type: z.literal('binary'),
  data: bytesSchema,
  mediaType: z.string(),
})
export const urlSourceSchema = z.strictObject({ type: z.literal('url'), url: z.string() })
export const mediaSourceSchema = z.union([binarySourceSchema, urlSourceSchema])
export const documentSourceSchema = z.strictObject({
  data: bytesSchema,
  mediaType: z.string(),
  fileName: z.string().min(1),
})

export function createUserContentSchema<M extends z.ZodType, D extends z.ZodType>(
  media: M,
  document: D,
) {
  return z.discriminatedUnion('type', [
    z.object({ type: z.literal('text'), text: z.string() }),
    z.object({ type: z.literal('image'), source: media }),
    z.object({ type: z.literal('video'), source: media }),
    z.object({ type: z.literal('document'), source: document }),
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
  ])
}

export const userContentBlockSchema = createUserContentSchema(
  mediaSourceSchema,
  documentSourceSchema,
)

export function createToolContentSchema<S extends z.ZodType>(source: S) {
  return z.discriminatedUnion('type', [
    z.object({ type: z.literal('text'), text: z.string() }),
    z.object({ type: z.literal('image'), source }),
    z.object({ type: z.literal('video'), source }),
  ])
}

export const base64SourceSchema = z.strictObject({
  mediaType: z.string(),
  data: z.string().refine((value) => {
    try {
      base64ToBytes(value)
      return true
    } catch {
      return false
    }
  }, 'Invalid base64 media'),
})
export const toolResultContentBlockSchema = createToolContentSchema(base64SourceSchema)

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

export const editReceiptsSchema = z.array(editReceiptSchema).refine(
  (items) => new Set(items.map((item) => item.operationId)).size === items.length,
  'Duplicate accepted edit operation IDs',
)

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

export function createClientFrameSchema<C extends z.ZodType>(content: C) {
  return z.discriminatedUnion('type', [
    z.object({
      type: z.literal('open'),
      provider: providerSelectionSchema,
      cwd: z.string(),
      sessionId: z.string()
    }),
    sendFrameSchema.extend({ content: z.array(content) }),
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
    steerFrameSchema.extend({ content: z.array(content) }),
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
}

export const clientFrameSchema = createClientFrameSchema(userContentBlockSchema)

export const sessionPhaseSchema = z.enum(['idle', 'running', 'compacting'])
export const queuedMessageSchema = z.object({
  id: z.string().min(1),
  text: z.string(),
  content: z.array(userContentBlockSchema),
})
