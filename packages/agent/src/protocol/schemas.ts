// The inbound wire boundary: client frames are declared here as zod schemas
// and `frames.ts` derives the `ClientFrame` type from them — one declaration,
// validated at AgentServer transport ingress. Shared types owned by other
// packages (core content blocks, provider selections) keep their hand-written
// types as the source of truth; their validators carry a `z.ZodType<T>`
// annotation so schema drift is a compile error, not a runtime surprise.
import { z } from 'zod'
import type {
  DocumentSource,
  ImageSource,
  Model,
  ModelSelection,
  ThinkingCapability,
  ThinkingConfig,
  UserContentBlock,
  VideoSource,
} from '@demicodes/core'
import type { FileExtension } from '@demicodes/core'
import type { ProviderSelection } from '@demicodes/provider'
import type { PortableJsonValue } from '@demicodes/utils'
import type { AgentMetadata, ModelSwitchApply } from '../types'

// ── shared-type validators ──────────────────────────────────────────

// The extension list is a closed core-owned set; the wire validates
// stringness and leaves membership to the catalog code that consumes it.
const fileExtensionSchema = z.custom<FileExtension>((value) => typeof value === 'string')

const thinkingCapabilitySchema: z.ZodType<ThinkingCapability> = z.discriminatedUnion('type', [
  z.object({ type: z.literal('adaptive'), efforts: z.array(z.string()), defaultEffort: z.string().nullable() }),
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
    defaultSummary: z.enum(['auto', 'concise', 'detailed', 'off', 'on']).nullable(),
  }),
  z.object({ type: z.literal('disabled') }),
])

const thinkingConfigSchema: z.ZodType<ThinkingConfig> = z.discriminatedUnion('type', [
  z.object({ type: z.literal('adaptive'), effort: z.string() }),
  z.object({ type: z.literal('budget'), budgetTokens: z.number() }),
  z.object({
    type: z.literal('effort'),
    effort: z.string(),
    summary: z.enum(['auto', 'concise', 'detailed', 'off', 'on']).nullable(),
  }),
  z.object({ type: z.literal('disabled') }),
])

const modelSchema: z.ZodType<Model> = z.object({
  id: z.string(),
  name: z.string(),
  contextWindow: z.number(),
  inputLimit: z.number().nullable(),
  thinking: z.array(thinkingCapabilitySchema),
  acceptedExtensions: z.array(fileExtensionSchema),
})

const modelSelectionSchema: z.ZodType<ModelSelection> = z.object({
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
  z.object({ type: z.literal('binary'), data: z.instanceof(Uint8Array), mediaType: z.string() }),
  z.object({ type: z.literal('url'), url: z.string() }),
])

const videoSourceSchema: z.ZodType<VideoSource> = z.union([
  z.object({ type: z.literal('binary'), data: z.instanceof(Uint8Array), mediaType: z.string() }),
  z.object({ type: z.literal('url'), url: z.string() }),
])

const documentSourceSchema: z.ZodType<DocumentSource> = z.object({
  data: z.instanceof(Uint8Array),
  mediaType: z.string(),
  fileName: z.string(),
})

export const userContentBlockSchema: z.ZodType<UserContentBlock> = z.discriminatedUnion('type', [
  z.object({ type: z.literal('text'), text: z.string() }),
  z.object({ type: z.literal('image'), source: imageSourceSchema }),
  z.object({ type: z.literal('video'), source: videoSourceSchema }),
  z.object({ type: z.literal('document'), source: documentSourceSchema }),
  z.object({ type: z.literal('reference'), reference: z.string() }),
])

const portableJsonValueSchema: z.ZodType<PortableJsonValue> = z.lazy(() =>
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

const metadataSchema: z.ZodType<AgentMetadata> = z.record(z.string(), portableJsonValueSchema)

const modelSwitchApplySchema: z.ZodType<ModelSwitchApply> = z.enum(['immediate', 'next_turn'])

// ── the client frames (single source of truth for ClientFrame) ──────

export const sendFrameSchema = z.object({
  type: z.literal('send'),
  messageId: z.string(),
  content: z.array(userContentBlockSchema),
  metadata: metadataSchema.optional(),
})

export const steerFrameSchema = z.object({ type: z.literal('steer'), steerId: z.string(), content: z.array(userContentBlockSchema) })

export const clientFrameSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('open'), provider: providerSelectionSchema, cwd: z.string(), sessionId: z.string() }),
  sendFrameSchema,
  z.object({ type: z.literal('dequeue_message'), messageId: z.string() }),
  z.object({ type: z.literal('send_queued_message'), messageId: z.string() }),
  z.object({ type: z.literal('steer_queued_message'), messageId: z.string(), steerId: z.string() }),
  z.object({ type: z.literal('clear_message_queue') }),
  steerFrameSchema,
  z.object({ type: z.literal('cancel_pending_steer'), steerId: z.string() }),
  z.object({ type: z.literal('set_provider'), provider: providerSelectionSchema, apply: modelSwitchApplySchema.optional() }),
  z.object({ type: z.literal('abort') }),
  z.object({ type: z.literal('abort_subagents') }),
  z.object({ type: z.literal('retry'), metadata: metadataSchema.optional() }),
  z.object({ type: z.literal('resume'), metadata: metadataSchema.optional() }),
  z.object({ type: z.literal('compact'), metadata: metadataSchema.optional() }),
  z.object({
    type: z.literal('shell_write'),
    commandId: z.string(),
    stdin: z.string(),
    metadata: metadataSchema.optional(),
  }),
  z.object({ type: z.literal('list_conversations'), cwd: z.string() }),
  // Requests a fresh transcript_reset; sent by the client when it detects a
  // revision gap in the patch stream (defensive resync, transports are ordered).
  z.object({ type: z.literal('sync_transcript') }),
  z.object({ type: z.literal('close') }),
])
