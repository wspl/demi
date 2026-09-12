import { z } from 'zod'

const counter = z.number().int().nonnegative()
export const googleUsageSchema = z.looseObject({
  promptTokenCount: counter.optional(),
  candidatesTokenCount: counter.optional(),
  thoughtsTokenCount: counter.optional(),
  cachedContentTokenCount: counter.optional(),
}).refine((usage) => usage.promptTokenCount === undefined
  || (usage.cachedContentTokenCount ?? 0) <= usage.promptTokenCount, {
  path: ['cachedContentTokenCount'], message: 'Cached count exceeds prompt count',
})
export type GoogleUsage = z.infer<typeof googleUsageSchema>

const partSchema = z.looseObject({
  text: z.string().optional(),
  thought: z.boolean().optional(),
  thoughtSignature: z.string().optional(),
  functionCall: z.looseObject({
    id: z.string().min(1).optional(),
    name: z.string().min(1).max(128).regex(/^[a-zA-Z0-9_-]+$/),
    args: z.record(z.string(), z.unknown()).optional(),
  }).optional(),
}).refine((part) => part.text === undefined || part.functionCall === undefined, {
  path: ['functionCall'], message: 'Part has multiple data variants',
})

export const googleResponseSchema = z.looseObject({
  candidates: z.array(z.looseObject({
    index: counter.optional(),
    content: z.looseObject({ parts: z.array(partSchema).optional() }).optional(),
    finishReason: z.string().min(1).optional(),
  })).optional(),
  usageMetadata: googleUsageSchema.optional(),
  promptFeedback: z.looseObject({ blockReason: z.string().min(1).optional() }).optional(),
  error: z.looseObject({
    code: counter.optional(),
    message: z.string(),
    status: z.string().min(1),
  }).optional(),
})
