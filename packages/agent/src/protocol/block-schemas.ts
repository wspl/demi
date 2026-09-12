import { z } from 'zod'
import {
  modelSelectionSchema,
  userContentBlockSchema,
  toolResultContentBlockSchema,
} from './schemas'

const meta = {
  id: z.string(),
  createdAt: z.string(),
  model: modelSelectionSchema,
}
export const diagnosticsSchema = z.object({
  source: z.enum(['http', 'stream', 'transport', 'unknown']),
  clientRequestId: z.string().optional(),
  providerRequestId: z.string().optional(),
  providerResponseId: z.string().optional(),
  providerCode: z.string().optional(),
  httpStatus: z.number().nonnegative().optional(),
})
export const usageSchema = z.object({
  inputTokens: z.number().int().nonnegative(),
  outputTokens: z.number().int().nonnegative(),
  cacheReadTokens: z.number().int().nonnegative(),
  cacheWriteTokens: z.number().int().nonnegative(),
})
export function createBlockSchema<U extends z.ZodType, T extends z.ZodType>(
  userContent: U,
  toolContent: T,
) {
  return z.discriminatedUnion('type', [
    z.object({
      ...meta,
      type: z.literal('user'),
      turnId: z.string(),
      content: z.array(userContent),
      preamble: z.string().nullable(),
      resolvedContent: z.array(userContent).optional(),
      hidden: z.boolean().optional(),
    }),
    z.object({
      ...meta,
      type: z.literal('resume'),
      turnId: z.string(),
    }),
    z.object({
      ...meta,
      type: z.literal('steer'),
      turnId: z.string(),
      content: z.array(userContent),
      hidden: z.boolean().optional(),
    }),
    z.object({
      ...meta,
      type: z.literal('thinking'),
      text: z.string(),
      signature: z.string().nullable(),
    }),
    z.object({
      ...meta,
      type: z.literal('redacted_thinking'),
      data: z.string(),
    }),
    z.object({
      ...meta,
      type: z.literal('text'),
      text: z.string(),
      forkable: z.literal(true).optional(),
    }),
    z.object({
      ...meta,
      type: z.literal('tool_call'),
      toolUseId: z.string(),
      toolName: z.string(),
      input: z.string(),
      status: z.enum(['executing', 'completed', 'error']),
      streamingOutput: z.array(toolContent),
      output: z.array(toolContent),
      view: z.unknown().refine((value): boolean => value !== undefined, 'View must be present'),
    }),
    z.object({
      ...meta,
      type: z.literal('response'),
      usage: usageSchema,
    }),
    z.object({
      ...meta,
      type: z.literal('error'),
      message: z.string(),
      code: z.string().nullable(),
      diagnostics: diagnosticsSchema.optional(),
    }),
    z.object({
      ...meta,
      type: z.literal('abort'),
      isResumed: z.boolean(),
    }),
    z.object({
      ...meta,
      type: z.literal('compaction_boundary'),
      summary: z.string(),
      summaryTokens: z.number().nonnegative(),
    }),
    z.object({
      ...meta,
      type: z.literal('compaction_marker'),
      boundaryId: z.string(),
      compactedTokens: z.number().nonnegative(),
    }),
    z.object({
      type: z.literal('extension_state_snapshot'),
      id: z.string(),
      createdAt: z.string(),
      extensionName: z.string(),
      state: z.unknown().refine((value): boolean => value !== undefined, 'State must be present'),
    }),
  ])
}

export const blockSchema = createBlockSchema(userContentBlockSchema, toolResultContentBlockSchema)
