import { z } from 'zod'
import { parseProviderData } from '@demicodes/provider'

const identifier = z.string().min(1)
const requestId = z.union([identifier, z.number().int()])
const count = z.number().int().nonnegative()
const usageCountsSchema = z.looseObject({
  input_tokens: count.optional(),
  output_tokens: count.optional(),
  cache_read_input_tokens: count.optional(),
  cache_creation_input_tokens: count.optional(),
  // The legacy injected transport uses Demi's camel-case usage fields.
  inputTokens: count.optional(),
  outputTokens: count.optional(),
  cacheReadTokens: count.optional(),
  cacheWriteTokens: count.optional(),
})
const usageSchema = usageCountsSchema.extend({
  iterations: z.array(usageCountsSchema).optional(),
})
const contentSchema = z.discriminatedUnion('type', [
  z.looseObject({ type: z.literal('text'), text: z.string() }),
  z.looseObject({
    type: z.literal('thinking'), thinking: z.string(), signature: z.string().optional(),
  }),
  z.looseObject({ type: z.literal('redacted_thinking'), data: z.string() }),
  z.looseObject({
    type: z.literal('tool_use'), id: identifier, name: identifier, input: z.unknown(),
  }),
])
const streamEventSchema = z.discriminatedUnion('type', [
  z.looseObject({
    type: z.literal('content_block_start'),
    index: count.optional(),
    content_block: contentSchema,
  }),
  z.looseObject({
    type: z.literal('content_block_delta'),
    index: count.optional(),
    delta: z.discriminatedUnion('type', [
      z.looseObject({ type: z.literal('text_delta'), text: z.string() }),
      z.looseObject({ type: z.literal('thinking_delta'), thinking: z.string() }),
      z.looseObject({ type: z.literal('signature_delta'), signature: z.string() }),
      z.looseObject({ type: z.literal('input_json_delta'), partial_json: z.string() }),
      z.looseObject({ type: z.literal('citations_delta') }),
    ]),
  }),
  z.looseObject({ type: z.literal('content_block_stop'), index: count.optional() }),
  z.looseObject({ type: z.literal('message_start'), message: z.looseObject({
    content: z.array(contentSchema), usage: usageCountsSchema.optional(),
  }) }),
  z.looseObject({ type: z.literal('message_delta'), usage: usageCountsSchema.optional() }),
  z.looseObject({ type: z.literal(['message_stop', 'ping']) }),
  z.looseObject({ type: z.literal('error'), error: z.looseObject({
    type: identifier, message: z.string(),
  }) }),
])

const paramsSchema = z.looseObject({
  name: identifier.optional(),
  arguments: z.unknown().optional(),
  input: z.unknown().optional(),
  _meta: z.looseObject({ 'claudecode/toolUseId': identifier.optional() }).optional(),
})
const rpcMessageSchema = z.object({
  id: requestId.optional(),
  method: identifier,
  params: paramsSchema.optional(),
}).superRefine((request, ctx) => {
  if (request.id === undefined && !request.method.startsWith('notifications/')) {
    ctx.addIssue({ code: 'custom', path: ['id'], message: 'Request ID is required' })
  }
  if (request.method === 'tools/call' && !request.params?.name) {
    ctx.addIssue({ code: 'custom', path: ['params', 'name'], message: 'Tool name is required' })
  }
})
const sdkControlSchema = z.object({
  type: z.literal('control_request'),
  request_id: identifier,
  request: z.looseObject({
    subtype: z.literal('mcp_message'),
    server_name: z.literal('main'),
    message: rpcMessageSchema,
  }),
})
const legacyControlSchema = rpcMessageSchema.safeExtend({ type: z.literal('control_request') })

export const claudeOutputSchema = z.union([
  z.discriminatedUnion('type', [
    z.looseObject({ type: z.literal('assistant'), message: z.looseObject({
      content: z.array(contentSchema), usage: usageCountsSchema.optional(),
    }) }),
    z.looseObject({ type: z.literal('stream_event'), event: streamEventSchema }),
    z.looseObject({
      type: z.literal('result'),
      is_error: z.boolean().optional(),
      result: z.string().optional(),
      errors: z.array(z.string()).optional(),
      usage: usageSchema.optional(),
    }),
    z.looseObject({
      type: z.literal('error'), message: z.string(), code: z.string().nullable().optional(),
    }),
    z.looseObject({ type: z.literal('control_response'), response: z.discriminatedUnion('subtype', [
      z.looseObject({ subtype: z.literal('success'), request_id: identifier }),
      z.looseObject({ subtype: z.literal('error'), request_id: identifier, error: z.string() }),
    ]) }),
    // These CLI messages do not supply Demi content or execution requests.
    z.looseObject({ type: z.literal([
      'system', 'user', 'tool_progress', 'tool_use_summary', 'auth_status',
      'rate_limit_event', 'prompt_suggestion',
    ]) }),
  ]),
  sdkControlSchema,
  legacyControlSchema,
])

export type ClaudeOutputMessage = z.infer<typeof claudeOutputSchema>
export type ClaudeContent = z.infer<typeof contentSchema>
export type ClaudeStreamEvent = z.infer<typeof streamEventSchema>
export type ClaudeUsage = z.infer<typeof usageSchema>
export type ClaudeRpcParams = z.infer<typeof paramsSchema>

export function parseClaudeOutputMessage(value: unknown): ClaudeOutputMessage {
  return parseProviderData(claudeOutputSchema, value, 'Claude Code output')
}

/** All transport implementations, including injected transports, enter here. */
export async function readClaudeMessage(
  iterator: AsyncIterator<unknown>
): Promise<IteratorResult<ClaudeOutputMessage>> {
  const next = await iterator.next()
  if (next.done) {
    return { done: true, value: undefined }
  }
  return { done: false, value: parseClaudeOutputMessage(next.value) }
}
