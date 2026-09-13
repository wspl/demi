/**
 * The Claude Code CLI's stdout stream-json protocol, as Demi reads it: one
 * schema for the message types Demi maps, and a pure mapping from a decoded
 * message to provider events.
 *
 * A message, content block, stream event or delta of a type Demi does not read
 * decodes to `null` and is skipped, so a CLI release that adds one does not
 * break a run. A type Demi does read whose payload is malformed is a protocol
 * error naming the offending field.
 */
import { z } from 'zod'
import {
  normalizeErrorCode,
  reportedStringSchema,
  taggedUnion,
  tokenCountSchema,
  type ProviderEvent
} from '@demicodes/provider'
import type { TokenUsage } from '@demicodes/core'

/**
 * The token counts a `result` message carries, in the API's snake_case
 * spelling and in the camelCase spelling of Demi's own `TokenUsage`.
 */
const usageCountsSchema = z.looseObject({
  input_tokens: tokenCountSchema,
  output_tokens: tokenCountSchema,
  cache_read_input_tokens: tokenCountSchema,
  cache_creation_input_tokens: tokenCountSchema,
  inputTokens: tokenCountSchema,
  outputTokens: tokenCountSchema,
  cacheReadTokens: tokenCountSchema,
  cacheWriteTokens: tokenCountSchema,
})
type ClaudeUsageCounts = z.infer<typeof usageCountsSchema>

/**
 * `result.usage` sums every API call the CLI made within the turn (the initial
 * call plus one per tool result), while `iterations` lists those calls
 * individually.
 */
const resultUsageSchema = usageCountsSchema.extend({
  iterations: z.array(usageCountsSchema).optional(),
})

/** The arguments an MCP `tools/call` request carries. */
const mcpParamsSchema = z.looseObject({
  name: z.string().optional(),
  arguments: z.unknown().optional(),
  input: z.unknown().optional(),
  _meta: z.looseObject({
    'claudecode/toolUseId': z.string().min(1).optional(),
  }).optional(),
})
export type ClaudeMcpParams = z.infer<typeof mcpParamsSchema>

/** The JSON-RPC request an SDK-MCP `control_request` wraps. */
const mcpMessageSchema = z.looseObject({
  id: z.union([z.string(), z.number()]).nullish(),
  method: z.string().optional(),
  params: mcpParamsSchema.optional(),
})

/**
 * A `control_request` in either protocol: the SDK-MCP one wraps a JSON-RPC
 * message the response must quote `request_id` back to, the legacy one *is*
 * the JSON-RPC request.
 */
const controlRequestSchema = z.looseObject({
  type: z.literal('control_request'),
  request_id: z.string().optional(),
  request: z.looseObject({
    subtype: z.string().optional(),
    server_name: z.string().optional(),
    message: mcpMessageSchema.optional(),
  }).optional(),
  id: z.union([z.string(), z.number()]).nullish(),
  method: z.string().optional(),
  params: mcpParamsSchema.optional(),
})

/** One block of an assistant message's content. */
const contentBlockSchema = taggedUnion({
  text: z.looseObject({
    type: z.literal('text'),
    text: z.string().default(''),
  }),
  thinking: z.looseObject({
    type: z.literal('thinking'),
    thinking: z.string().optional(),
    text: z.string().optional(),
    signature: z.string().optional(),
  }),
  redacted_thinking: z.looseObject({
    type: z.literal('redacted_thinking'),
    data: z.string().default(''),
  }),
  tool_use: z.looseObject({
    type: z.literal('tool_use'),
    id: z.union([z.string(), z.number()]).nullish(),
    name: z.string().optional(),
    input: z.unknown().optional(),
  }),
})
type ClaudeContentBlock = NonNullable<z.infer<typeof contentBlockSchema>>
type ClaudeToolUseBlock = Extract<ClaudeContentBlock, { type: 'tool_use' }>

/** One increment of a streamed content block. */
const streamDeltaSchema = taggedUnion({
  text_delta: z.looseObject({
    type: z.literal('text_delta'),
    text: z.string().default(''),
  }),
  thinking_delta: z.looseObject({
    type: z.literal('thinking_delta'),
    thinking: z.string().default(''),
  }),
  signature_delta: z.looseObject({
    type: z.literal('signature_delta'),
    signature: z.string().default(''),
  }),
})

/**
 * The streaming events Demi reads. `message_stop` maps to no event, but it
 * closes the turn's batch of tool calls, so the provider needs to see it.
 */
const streamEventSchema = taggedUnion({
  content_block_start: z.looseObject({
    type: z.literal('content_block_start'),
    content_block: contentBlockSchema.optional(),
  }),
  content_block_delta: z.looseObject({
    type: z.literal('content_block_delta'),
    delta: streamDeltaSchema.optional(),
  }),
  message_stop: z.looseObject({ type: z.literal('message_stop') }),
})
type ClaudeStreamEvent = NonNullable<z.infer<typeof streamEventSchema>>

/** One line of the CLI's stdout stream. */
export const claudeStdoutMessageSchema = taggedUnion({
  assistant: z.looseObject({
    type: z.literal('assistant'),
    message: z.looseObject({
      content: z.array(contentBlockSchema).optional(),
    }).optional(),
  }),
  stream_event: z.looseObject({
    type: z.literal('stream_event'),
    event: streamEventSchema.optional(),
  }),
  control_request: controlRequestSchema,
  control_response: z.looseObject({
    type: z.literal('control_response'),
    response: z.looseObject({
      subtype: z.string().optional(),
      request_id: z.string().optional(),
    }).optional(),
  }),
  result: z.looseObject({
    type: z.literal('result'),
    is_error: z.boolean().optional(),
    // Error prose the CLI reports; a malformed one reads as absent so the
    // failure still surfaces as an error instead of as a protocol fault.
    result: reportedStringSchema,
    errors: z.array(reportedStringSchema).optional().catch(undefined),
    usage: resultUsageSchema.optional(),
  }),
  error: z.looseObject({
    type: z.literal('error'),
    message: reportedStringSchema,
    code: reportedStringSchema,
  }),
})
export type ClaudeStdoutMessage =
  NonNullable<z.infer<typeof claudeStdoutMessageSchema>>
type ClaudeControlRequestMessage =
  Extract<ClaudeStdoutMessage, { type: 'control_request' }>
type ClaudeResultMessage = Extract<ClaudeStdoutMessage, { type: 'result' }>

export interface OutputMapping {
  events: ProviderEvent[]
  controlRequest?: ClaudeControlRequest
  terminal: boolean
}

export interface ClaudeControlRequest {
  protocol: 'legacy' | 'sdk-mcp'
  outerRequestId?: string
  serverName?: string
  id: string | number
  toolUseId?: string
  method: string
  params?: ClaudeMcpParams
}

export interface ClaudeOutputMapOptions {
  ignoreAssistantContent?: boolean
  ignoreAssistantToolUse?: boolean
}

/**
 * Decodes one stdout line. Returns null for a message type Demi does not read;
 * throws when a type it reads arrives malformed.
 */
export function decodeClaudeStdoutMessage(
  line: unknown
): ClaudeStdoutMessage | null {
  return claudeStdoutMessageSchema.parse(line)
}

export function mapClaudeStdoutMessage(
  message: ClaudeStdoutMessage | null,
  options: ClaudeOutputMapOptions = {}
): OutputMapping {
  if (!message)
    return { events: [], terminal: false }

  switch (message.type) {
    case 'assistant':
      return {
        events: mapContentBlocks(message.message?.content ?? [], options),
        terminal: false,
      }
    case 'stream_event':
      return { events: mapStreamEvent(message.event), terminal: false }
    case 'control_request': {
      const controlRequest = mapControlRequest(message)
      if (!controlRequest)
        return { events: [], terminal: false }
      return { events: [], controlRequest, terminal: false }
    }
    case 'result':
      return { events: mapResult(message), terminal: true }
    case 'error':
      return {
        events: [errorEvent(
          message.message ?? 'Claude Code error',
          message.code
        )],
        terminal: false,
      }
    case 'control_response':
      // Answers a request Demi sent; the caller matches it by request id.
      return { events: [], terminal: false }
  }
}

export function controlRequestToToolCall(
  request: ClaudeControlRequest
): ProviderEvent | null {
  if (request.method !== 'tools/call')
    return null
  const params = request.params
  if (!params?.name)
    return null
  return {
    type: 'tool_call_requested',
    toolUseId: request.toolUseId ?? String(request.id),
    toolName: stripMcpToolPrefix(params.name),
    input: params.arguments ?? params.input ?? {},
  }
}

function mapContentBlocks(
  content: Array<ClaudeContentBlock | null>,
  options: ClaudeOutputMapOptions,
): ProviderEvent[] {
  const events: ProviderEvent[] = []
  for (const block of content) {
    if (!block)
      continue
    if (block.type === 'tool_use') {
      if (!options.ignoreAssistantToolUse)
        events.push(toolUseEvent(block))
      continue
    }
    if (options.ignoreAssistantContent)
      continue
    if (block.type === 'text') {
      events.push({ type: 'text_delta', text: block.text })
      continue
    }
    if (block.type === 'redacted_thinking') {
      events.push({ type: 'redacted_thinking', data: block.data })
      continue
    }
    // The only block type left is `thinking`.
    events.push({ type: 'thinking_start' })
    events.push({
      type: 'thinking_delta',
      text: block.thinking ?? block.text ?? ''
    })
    if (block.signature !== undefined)
      events.push({ type: 'thinking_signature', signature: block.signature })
  }
  return events
}

function toolUseEvent(block: ClaudeToolUseBlock): ProviderEvent {
  if (block.id == null || !block.name) {
    return {
      type: 'error',
      message: 'Invalid tool_use block from Claude Code',
      code: null,
    }
  }
  return {
    type: 'tool_call_requested',
    toolUseId: String(block.id),
    toolName: stripMcpToolPrefix(block.name),
    input: block.input ?? {},
  }
}

function mapStreamEvent(
  event: ClaudeStreamEvent | null | undefined
): ProviderEvent[] {
  if (!event)
    return []

  if (event.type === 'content_block_start') {
    const block = event.content_block
    if (block?.type === 'thinking')
      return [{ type: 'thinking_start' }]
    if (block?.type === 'text')
      return [{ type: 'text_delta', text: block.text }]
    return []
  }

  if (event.type === 'content_block_delta') {
    const delta = event.delta
    if (delta?.type === 'text_delta')
      return [{ type: 'text_delta', text: delta.text }]
    if (delta?.type === 'thinking_delta')
      return [{ type: 'thinking_delta', text: delta.thinking }]
    if (delta?.type === 'signature_delta')
      return [{ type: 'thinking_signature', signature: delta.signature }]
    return []
  }

  return []
}

function mapControlRequest(
  message: ClaudeControlRequestMessage
): ClaudeControlRequest | null {
  const request = message.request
  if (message.request_id !== undefined && request !== undefined) {
    if (request.subtype !== 'mcp_message')
      return null
    if (request.server_name !== 'main')
      return null
    const inner = request.message
    if (!inner?.method)
      return null
    return {
      protocol: 'sdk-mcp',
      outerRequestId: message.request_id,
      serverName: request.server_name,
      id: inner.id ?? 0,
      toolUseId: inner.params?._meta?.['claudecode/toolUseId'],
      method: inner.method,
      params: inner.params,
    }
  }

  if (message.id == null || !message.method)
    return null
  return {
    protocol: 'legacy',
    id: message.id,
    method: message.method,
    params: message.params,
  }
}

function mapResult(message: ClaudeResultMessage): ProviderEvent[] {
  const events: ProviderEvent[] = []
  if (message.is_error === true)
    events.push(errorEvent(resultErrorMessage(message)))
  events.push({ type: 'response', usage: resultUsage(message.usage) })
  return events
}

function resultErrorMessage(message: ClaudeResultMessage): string {
  const parts: string[] = []
  const result = message.result?.trim()
  if (result)
    parts.push(result)
  for (const error of message.errors ?? []) {
    const text = error?.trim()
    if (text)
      parts.push(text)
  }
  return parts.join('\n') || 'Claude Code returned an error'
}

/**
 * The usage of the turn's *final* API request, which is what the response
 * contract requires: the last `iterations` entry when the CLI reported them,
 * and otherwise the top-level counts.
 */
function resultUsage(
  usage: z.infer<typeof resultUsageSchema> | undefined
): TokenUsage {
  const iterations = usage?.iterations ?? []
  return tokenUsage(iterations[iterations.length - 1] ?? usage)
}

function tokenUsage(counts: ClaudeUsageCounts | undefined): TokenUsage {
  return {
    inputTokens: counts?.input_tokens ?? counts?.inputTokens ?? 0,
    outputTokens: counts?.output_tokens ?? counts?.outputTokens ?? 0,
    cacheReadTokens: counts?.cache_read_input_tokens
      ?? counts?.cacheReadTokens ?? 0,
    cacheWriteTokens: counts?.cache_creation_input_tokens
      ?? counts?.cacheWriteTokens ?? 0,
  }
}

function errorEvent(message: string, code?: string): ProviderEvent {
  return {
    type: 'error',
    message,
    code: normalizeErrorCode(code ?? null, message),
  }
}

function stripMcpToolPrefix(name: string): string {
  const match = /^mcp__[^_]+__(.+)$/.exec(name)
  return match?.[1] ?? name
}
