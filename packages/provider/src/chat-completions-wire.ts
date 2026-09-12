import { z } from 'zod'

const counter = z.number().int().nonnegative()
const identifier = z.string().min(1)
export const chatToolDeltaSchema = z.looseObject({
  index: counter,
  id: identifier.optional(),
  type: z.literal('function').optional(),
  function: z.looseObject({
    name: identifier.optional(),
    arguments: z.string().optional(),
  }).optional(),
})
export type ChatToolDelta = z.infer<typeof chatToolDeltaSchema>

const deltaSchema = z.looseObject({
  role: z.string().optional(),
  content: z.string().nullable().optional(),
  reasoning_content: z.string().nullable().optional(),
  tool_calls: z.array(chatToolDeltaSchema).nullable().optional(),
})
const choiceSchema = z.looseObject({
  index: counter.optional(),
  delta: deltaSchema.nullable().optional(),
  finish_reason: z.string().min(1).nullable().optional(),
}).refine((choice) => choice.delta != null || choice.finish_reason != null, {
  path: ['delta'], message: 'Nonterminal choice needs a delta',
})
const usageSchema = z.looseObject({
  prompt_tokens: counter.optional(),
  completion_tokens: counter.optional(),
  prompt_tokens_details: z.looseObject({ cached_tokens: counter.optional() }).optional(),
}).refine((usage) => (usage.prompt_tokens_details?.cached_tokens ?? 0)
  <= (usage.prompt_tokens ?? 0), { path: ['prompt_tokens_details', 'cached_tokens'] })
export type ChatUsage = z.infer<typeof usageSchema>

export const chatCompletionChunkSchema = z.looseObject({
  choices: z.array(choiceSchema).optional(),
  usage: usageSchema.nullable().optional(),
  error: z.looseObject({
    code: z.string().nullable().optional(),
    type: z.string().optional(),
    message: z.string().optional(),
  }).optional(),
}).refine((chunk) => chunk.choices !== undefined || chunk.error !== undefined, {
  path: ['choices'], message: 'Chunk needs choices or an error',
}).refine((chunk) => {
  const indexes = chunk.choices?.map((choice, position) => choice.index ?? position) ?? []
  return new Set(indexes).size === indexes.length
}, { path: ['choices'], message: 'Duplicate choice index' })
