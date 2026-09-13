/**
 * The OpenAI Chat Completions streaming format, as Demi reads it. Shared by
 * every adapter that speaks it (the OpenAI-compatible HTTP providers).
 *
 * These schemas describe only what Demi consumes; `looseObject` keeps every
 * other field the vendor sends. A chunk carries no type tag, so there is
 * nothing to ignore: a malformed chunk is a protocol error naming the field.
 */
import { z } from 'zod'
import { tokenUsageWithCachedInput } from './usage'
import { reportedStringSchema, tokenCountSchema } from './vendor-schema'
import type { TokenUsage } from '@demicodes/core'

export const chatCompletionsUsageSchema = z.looseObject({
  prompt_tokens: tokenCountSchema,
  completion_tokens: tokenCountSchema,
  prompt_tokens_details: z.looseObject({ cached_tokens: tokenCountSchema })
    .optional(),
})
export type ChatCompletionsUsage = z.infer<typeof chatCompletionsUsageSchema>

/** The token usage the final chunk of a stream reports. */
export function tokenUsageFromChatCompletionsUsage(
  usage: ChatCompletionsUsage | null | undefined
): TokenUsage {
  return tokenUsageWithCachedInput({
    inputTokens: usage?.prompt_tokens,
    outputTokens: usage?.completion_tokens,
    cachedTokens: usage?.prompt_tokens_details?.cached_tokens,
  })
}

/**
 * One tool call increment. `index` groups the increments of one call; `id` and
 * `function.name` arrive once, `function.arguments` arrives in pieces.
 */
export const chatCompletionToolCallDeltaSchema = z.looseObject({
  index: z.number().optional(),
  id: z.string().optional(),
  function: z.looseObject({
    name: z.string().optional(),
    arguments: z.string().optional(),
  }).optional(),
})
export type ChatCompletionToolCallDelta =
  z.infer<typeof chatCompletionToolCallDeltaSchema>

/** A choice's increment. Vendors spell "nothing this chunk" as null. */
export const chatCompletionDeltaSchema = z.looseObject({
  content: z.string().nullable().optional(),
  reasoning_content: z.string().nullable().optional(),
  tool_calls: z.array(chatCompletionToolCallDeltaSchema).optional(),
})
export type ChatCompletionDelta = z.infer<typeof chatCompletionDeltaSchema>

export const chatCompletionChoiceSchema = z.looseObject({
  delta: chatCompletionDeltaSchema.optional(),
  finish_reason: z.string().nullable().optional(),
})
export type ChatCompletionChoice = z.infer<typeof chatCompletionChoiceSchema>

export const chatCompletionChunkSchema = z.looseObject({
  choices: z.array(chatCompletionChoiceSchema).optional(),
  // With `stream_options.include_usage`, only the last chunk carries counts;
  // the ones before it spell the absence as null.
  usage: chatCompletionsUsageSchema.nullable().optional(),
  error: z.looseObject({
    message: reportedStringSchema,
    code: reportedStringSchema,
    type: reportedStringSchema,
  }).optional(),
})
export type ChatCompletionChunk = z.infer<typeof chatCompletionChunkSchema>

/**
 * Decodes one streamed chunk. The `[DONE]` sentinel is not JSON and never
 * reaches here: the caller ends its stream on it.
 */
export function decodeChatCompletionChunk(raw: unknown): ChatCompletionChunk {
  return chatCompletionChunkSchema.parse(raw)
}
