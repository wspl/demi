import { z } from 'zod'
import { parseProviderJson, ProviderDataError, type ServerSentEvent } from '@demicodes/provider'

const counter = z.number().int().nonnegative()
const objectInput = z.record(z.string(), z.unknown())
export const anthropicUsageSchema = z.looseObject({
  input_tokens: counter.optional(),
  output_tokens: counter.optional(),
  cache_read_input_tokens: counter.optional(),
  cache_creation_input_tokens: counter.optional(),
})
export type AnthropicUsage = z.infer<typeof anthropicUsageSchema>

function ignoredVariant(knownTypes: readonly string[]) {
  return z.looseObject({
    type: z.string().min(1).refine((type) => !knownTypes.includes(type)),
  }).transform(() => ({ type: 'ignored' as const }))
}

const knownBlockSchema = z.discriminatedUnion('type', [
  z.looseObject({ type: z.literal('text'), text: z.string() }),
  z.looseObject({
    type: z.literal('thinking'),
    thinking: z.string().optional(),
    signature: z.string().optional(),
  }),
  z.looseObject({ type: z.literal('redacted_thinking'), data: z.string() }),
  z.looseObject({
    type: z.literal('tool_use'),
    id: z.string().min(1),
    name: z.string().min(1),
    input: objectInput,
  }),
])
const blockSchema = z.union([
  knownBlockSchema,
  ignoredVariant(knownBlockSchema.options.map((option) => option.shape.type.value)),
])

export type AnthropicResponseBlock = z.infer<typeof blockSchema>

const knownDeltaSchema = z.discriminatedUnion('type', [
  z.looseObject({ type: z.literal('text_delta'), text: z.string() }),
  z.looseObject({ type: z.literal('thinking_delta'), thinking: z.string() }),
  z.looseObject({ type: z.literal('signature_delta'), signature: z.string() }),
  z.looseObject({ type: z.literal('input_json_delta'), partial_json: z.string() }),
])
const deltaSchema = z.union([
  knownDeltaSchema,
  ignoredVariant(knownDeltaSchema.options.map((option) => option.shape.type.value)),
])

const knownEventSchema = z.discriminatedUnion('type', [
  z.looseObject({
    type: z.literal('message_start'),
    message: z.looseObject({ usage: anthropicUsageSchema.optional() }),
  }),
  z.looseObject({
    type: z.literal('content_block_start'),
    index: counter,
    content_block: blockSchema,
  }),
  z.looseObject({ type: z.literal('content_block_delta'), index: counter, delta: deltaSchema }),
  z.looseObject({ type: z.literal('content_block_stop'), index: counter }),
  z.looseObject({ type: z.literal('message_delta'), usage: anthropicUsageSchema.optional() }),
  z.looseObject({ type: z.literal('message_stop') }),
  z.looseObject({ type: z.literal('ping') }),
  z.looseObject({
    type: z.literal('error'),
    error: z.looseObject({ type: z.string().min(1), message: z.string() }),
  }),
])
export const anthropicEventSchema = z.union([
  knownEventSchema,
  ignoredVariant(knownEventSchema.options.map((option) => option.shape.type.value)),
])
export type AnthropicEvent = z.infer<typeof anthropicEventSchema>

export function parseAnthropicEvent(frame: ServerSentEvent): AnthropicEvent {
  const event = parseProviderJson(anthropicEventSchema, frame.data, 'Anthropic SSE event')
  if (event.type !== 'ignored' && frame.event && frame.event !== event.type) {
    throw new ProviderDataError('Anthropic SSE event', 'event name does not match payload type')
  }
  return event
}
