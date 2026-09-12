import { z } from 'zod'
import type { TokenUsage } from '@demicodes/core'

const identifier = z.string().min(1)
const textPartSchema = z.looseObject({ type: z.string().optional(), text: z.string() })
export const responsesReasoningItemSchema = z.looseObject({
  type: z.literal('reasoning'),
  id: identifier.optional(),
  summary: z.array(textPartSchema).optional(),
  content: z.array(textPartSchema).optional(),
  encrypted_content: z.string().nullable().optional(),
})
export const responsesMessageItemSchema = z.looseObject({
  type: z.literal('message'),
  id: identifier.optional(),
  role: z.string().optional(),
  content: z.array(z.discriminatedUnion('type', [
    z.looseObject({ type: z.literal('output_text'), text: z.string() }),
    z.looseObject({ type: z.literal('refusal'), refusal: z.string() }),
  ])),
  status: z.string().optional(),
  phase: z.string().optional(),
})
export const responsesFunctionCallItemSchema = z.looseObject({
  type: z.literal('function_call'),
  id: identifier,
  call_id: identifier,
  name: identifier,
  arguments: z.string(),
})
const outputItemSchema = z.discriminatedUnion('type', [
  responsesReasoningItemSchema,
  responsesMessageItemSchema,
  responsesFunctionCallItemSchema,
  // Hosted tools do not request execution through Demi's tool surface.
  z.looseObject({ type: z.literal([
    'file_search_call', 'web_search_call', 'code_interpreter_call',
    'image_generation_call', 'mcp_call', 'mcp_list_tools', 'mcp_approval_request',
  ]) }),
])

const count = z.number().int().nonnegative()
const usageSchema = z.looseObject({
  input_tokens: count.optional(),
  output_tokens: count.optional(),
  total_tokens: count.optional(),
  input_tokens_details: z.looseObject({ cached_tokens: count.optional() }).optional(),
}).refine((usage) => (usage.input_tokens_details?.cached_tokens ?? 0)
  <= (usage.input_tokens ?? 0), { path: ['input_tokens_details', 'cached_tokens'] })
export const responsesCompletedSchema = z.looseObject({
  id: identifier.optional(),
  status: z.string().optional(),
  end_turn: z.boolean().optional(),
  usage: usageSchema.nullable().optional(),
})
const errorSchema = z.looseObject({
  code: z.string().nullable().optional(),
  type: z.string().optional(),
  message: z.string().optional(),
  request_id: identifier.optional(),
  requestId: identifier.optional(),
})
export const responsesFailedSchema = z.looseObject({
  id: identifier.optional(),
  error: errorSchema.nullable().optional(),
  incomplete_details: z.looseObject({ reason: z.string() }).nullable().optional(),
})

export const responsesEventSchema = z.discriminatedUnion('type', [
  z.looseObject({
    type: z.literal(['response.output_item.added', 'response.output_item.done']),
    item: outputItemSchema,
  }),
  z.looseObject({
    type: z.literal([
      'response.reasoning_summary_text.delta', 'response.reasoning_text.delta',
      'response.output_text.delta',
    ]),
    delta: z.string(),
  }),
  z.looseObject({
    type: z.literal('response.function_call_arguments.delta'),
    item_id: identifier,
    delta: z.string(),
  }),
  z.looseObject({
    type: z.literal('response.function_call_arguments.done'),
    item_id: identifier,
    arguments: z.string(),
  }),
  z.looseObject({ type: z.literal('response.completed'), response: responsesCompletedSchema }),
  z.looseObject({
    type: z.literal(['response.failed', 'response.incomplete']),
    response: responsesFailedSchema,
  }),
  z.looseObject({
    type: z.literal('error'),
    code: z.string().nullable().optional(),
    message: z.string().optional(),
    error: errorSchema.optional(),
  }),
  // These events carry progress or copies of content consumed from other events.
  z.looseObject({ type: z.literal([
    'response.created', 'response.in_progress', 'response.queued',
    'response.content_part.added', 'response.content_part.done',
    'response.output_text.done', 'response.output_text.annotation.added',
    'response.refusal.delta', 'response.refusal.done',
    'response.reasoning_summary_part.added', 'response.reasoning_summary_part.done',
    'response.reasoning_summary_text.done', 'response.reasoning_text.done',
    'response.file_search_call.in_progress', 'response.file_search_call.searching',
    'response.file_search_call.completed', 'response.web_search_call.in_progress',
    'response.web_search_call.searching', 'response.web_search_call.completed',
    'response.code_interpreter_call.in_progress', 'response.code_interpreter_call.interpreting',
    'response.code_interpreter_call.completed', 'response.code_interpreter_call_code.delta',
    'response.code_interpreter_call_code.done',
  ]) }),
])

export type ResponsesStreamEvent = z.infer<typeof responsesEventSchema>
export type ResponsesOutputItem = z.infer<typeof outputItemSchema>
export type ResponsesReasoningItem = z.infer<typeof responsesReasoningItemSchema>
export type ResponsesMessageItem = z.infer<typeof responsesMessageItemSchema>
export type ResponsesFunctionCallItem = z.infer<typeof responsesFunctionCallItemSchema>
export type ResponsesCompleted = z.infer<typeof responsesCompletedSchema>
export type ResponsesFailed = z.infer<typeof responsesFailedSchema>


export function responsesUsage(response: ResponsesCompleted): TokenUsage {
  const usage = response.usage
  const inputTokens = usage?.input_tokens ?? 0
  const cachedTokens = usage?.input_tokens_details?.cached_tokens ?? 0
  return {
    inputTokens: inputTokens - cachedTokens,
    outputTokens: usage?.output_tokens ?? 0,
    cacheReadTokens: cachedTokens,
    cacheWriteTokens: 0,
  }
}
