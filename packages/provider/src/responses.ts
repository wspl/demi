/**
 * The OpenAI Responses API stream, as Demi reads it. Shared by every adapter
 * that speaks it (the Codex backend and the OpenAI Responses transport).
 *
 * These schemas describe only what Demi consumes; `looseObject` keeps every
 * other field the vendor sends, so a decoded item can be re-sent or stored
 * whole. Unknown event and item types decode to `null` and are ignored, while
 * a known type with a malformed payload is a protocol error naming the field.
 */
import { z } from 'zod'
import { tokenUsageWithCachedInput } from './usage'
import {
  reportedStringSchema,
  taggedUnion,
  tokenCountSchema
} from './vendor-schema'
import type { TokenUsage } from '@demicodes/core'

/** A vendor error object, as `response.failed` and `error` events carry it. */
export const responsesErrorSchema = z.looseObject({
  code: reportedStringSchema,
  type: reportedStringSchema,
  message: reportedStringSchema,
  request_id: reportedStringSchema,
  requestId: reportedStringSchema,
})
export type ResponsesError = z.infer<typeof responsesErrorSchema>

export const responsesUsageSchema = z.looseObject({
  input_tokens: tokenCountSchema,
  output_tokens: tokenCountSchema,
  input_tokens_details: z.looseObject({ cached_tokens: tokenCountSchema })
    .optional(),
})
export type ResponsesUsage = z.infer<typeof responsesUsageSchema>

/** The token usage a `response.completed` event reports. */
export function tokenUsageFromResponsesUsage(
  usage: ResponsesUsage | null | undefined
): TokenUsage {
  return tokenUsageWithCachedInput({
    inputTokens: usage?.input_tokens,
    outputTokens: usage?.output_tokens,
    cachedTokens: usage?.input_tokens_details?.cached_tokens,
  })
}

/** One part of a reasoning item's `summary` or `content`. */
const reasoningPartSchema = z.looseObject({
  type: z.string().optional(),
  text: z.string(),
})

/** One part of a message item's `content`; unknown parts decode to null. */
export const responsesMessagePartSchema = taggedUnion({
  output_text: z.looseObject({
    type: z.literal('output_text'),
    text: z.string(),
  }),
  refusal: z.looseObject({
    type: z.literal('refusal'),
    refusal: z.string(),
  }),
})
export type ResponsesMessagePart =
  NonNullable<z.infer<typeof responsesMessagePartSchema>>

export const responsesReasoningItemSchema = z.looseObject({
  type: z.literal('reasoning'),
  id: z.string().optional(),
  summary: z.array(reasoningPartSchema).optional(),
  content: z.array(reasoningPartSchema).optional(),
  encrypted_content: z.string().optional(),
})
export type ResponsesReasoningItem =
  z.infer<typeof responsesReasoningItemSchema>

export const responsesMessageItemSchema = z.looseObject({
  type: z.literal('message'),
  id: z.string().optional(),
  role: z.string().optional(),
  content: z.array(responsesMessagePartSchema).optional(),
  status: z.string().optional(),
})
export type ResponsesMessageItem = z.infer<typeof responsesMessageItemSchema>

export const responsesFunctionCallItemSchema = z.looseObject({
  type: z.literal('function_call'),
  id: z.string().optional(),
  call_id: z.string().optional(),
  name: z.string().optional(),
  arguments: z.string().optional(),
})
export type ResponsesFunctionCallItem =
  z.infer<typeof responsesFunctionCallItemSchema>

/** An output item of the three kinds Demi maps; any other decodes to null. */
export const responsesItemSchema = taggedUnion({
  reasoning: responsesReasoningItemSchema,
  message: responsesMessageItemSchema,
  function_call: responsesFunctionCallItemSchema,
})
export type ResponsesItem = NonNullable<z.infer<typeof responsesItemSchema>>

/** A `…delta` event: one increment of the text the vendor is streaming. */
function deltaEvent<T extends string>(type: T) {
  return z.looseObject({
    type: z.literal(type),
    delta: z.string(),
  })
}

/** The stream events Demi maps; any other event type decodes to null. */
export const responsesEventSchema = taggedUnion({
  'response.output_item.added': z.looseObject({
    type: z.literal('response.output_item.added'),
    item: responsesItemSchema.optional(),
  }),
  'response.output_item.done': z.looseObject({
    type: z.literal('response.output_item.done'),
    item: responsesItemSchema.optional(),
    item_id: z.string().optional(),
    call_id: z.string().optional(),
  }),
  'response.output_text.delta': deltaEvent('response.output_text.delta'),
  'response.reasoning_text.delta': deltaEvent('response.reasoning_text.delta'),
  'response.reasoning_summary_text.delta':
    deltaEvent('response.reasoning_summary_text.delta'),
  'response.function_call_arguments.delta': z.looseObject({
    type: z.literal('response.function_call_arguments.delta'),
    item_id: z.string().optional(),
    delta: z.string(),
  }),
  'response.function_call_arguments.done': z.looseObject({
    type: z.literal('response.function_call_arguments.done'),
    item_id: z.string().optional(),
    arguments: z.string(),
  }),
  'response.completed': z.looseObject({
    type: z.literal('response.completed'),
    response: z.looseObject({ usage: responsesUsageSchema.optional() })
      .optional(),
  }),
  'response.failed': z.looseObject({
    type: z.literal('response.failed'),
    response: z.looseObject({
      id: reportedStringSchema,
      error: responsesErrorSchema.optional(),
    }).optional(),
  }),
  'response.incomplete': z.looseObject({
    type: z.literal('response.incomplete'),
    response: z.looseObject({
      incomplete_details: z.looseObject({ reason: reportedStringSchema })
        .optional(),
    }).optional(),
  }),
  // The SSE stream sends {type:'error', message, code}; the Codex WebSocket
  // backend nests the same failure as {type:'error', error:{…}}.
  error: z.looseObject({
    type: z.literal('error'),
    message: reportedStringSchema,
    code: reportedStringSchema,
    error: responsesErrorSchema.optional(),
  }),
})
export type ResponsesEvent = NonNullable<z.infer<typeof responsesEventSchema>>

/**
 * Decodes one Responses stream event. Returns null for an event type Demi does
 * not map; throws when a mapped type arrives malformed.
 */
export function decodeResponsesEvent(raw: unknown): ResponsesEvent | null {
  return responsesEventSchema.parse(raw)
}
