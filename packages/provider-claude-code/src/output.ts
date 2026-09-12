import type { TokenUsage } from '@demicodes/core'
import type { ProviderEvent } from '@demicodes/provider'
import type {
  ClaudeContent, ClaudeOutputMessage, ClaudeRpcParams, ClaudeStreamEvent, ClaudeUsage,
} from './output-schemas'

export interface OutputMapping {
  events: ProviderEvent[]
  controlRequest?: ClaudeControlRequest
  terminal: boolean
}

export type ClaudeControlRequest = {
  id: string | number
  toolUseId?: string
  method: string
  params?: ClaudeRpcParams
} & ({ protocol: 'legacy' } | {
  protocol: 'sdk-mcp'
  outerRequestId: string
  serverName: 'main'
})

export interface ClaudeOutputMapOptions {
  ignoreAssistantContent?: boolean
  ignoreAssistantToolUse?: boolean
}

export function mapClaudeStdoutMessage(
  message: ClaudeOutputMessage,
  options: ClaudeOutputMapOptions = {}
): OutputMapping {
  const events: ProviderEvent[] = []
  switch (message.type) {
    case 'assistant':
      events.push(...mapContentArray(message.message.content, options))
      break
    case 'stream_event':
      events.push(...mapStreamEvent(message.event))
      break
    case 'control_request': {
      const request = parseControlRequest(message)
      if (request) {
        return { events, controlRequest: request, terminal: false }
      }
      break
    }
    case 'result':
      if (message.is_error) {
        const errorMessage = resultErrorMessage(message)
        events.push({ type: 'error', message: errorMessage, code: classifyProviderError(errorMessage) })
      }
      events.push({ type: 'response', usage: mapResultUsage(message.usage) })
      return { events, terminal: true }
    case 'error':
      events.push({
        type: 'error', message: message.message,
        code: message.code ?? classifyProviderError(message.message),
      })
      break
  }
  return { events, terminal: false }
}

export function controlRequestToToolCall(request: ClaudeControlRequest): ProviderEvent | null {
  if (request.method !== 'tools/call') {
    return null
  }
  // The wire schema requires a name for tools/call; other methods have optional params.
  const name = request.params?.name
  if (!name) {
    throw new Error('Validated tools/call request is missing its name')
  }
  return {
    type: 'tool_call_requested',
    toolUseId: request.toolUseId ?? String(request.id),
    toolName: stripMcpToolPrefix(name),
    input: request.params?.arguments !== undefined
      ? request.params.arguments
      : request.params?.input !== undefined ? request.params.input : {},
  }
}

function mapContentArray(content: ClaudeContent[], options: ClaudeOutputMapOptions): ProviderEvent[] {
  const events: ProviderEvent[] = []
  for (const block of content) {
    if (block.type === 'tool_use') {
      if (!options.ignoreAssistantToolUse) {
        events.push({
          type: 'tool_call_requested', toolUseId: block.id,
          toolName: stripMcpToolPrefix(block.name), input: block.input,
        })
      }
      continue
    }
    if (options.ignoreAssistantContent) {
      continue
    }
    switch (block.type) {
      case 'text':
        events.push({ type: 'text_delta', text: block.text })
        break
      case 'thinking':
        events.push({ type: 'thinking_start' })
        events.push({ type: 'thinking_delta', text: block.thinking })
        if (block.signature !== undefined) {
          events.push({ type: 'thinking_signature', signature: block.signature })
        }
        break
      case 'redacted_thinking':
        events.push({ type: 'redacted_thinking', data: block.data })
        break
    }
  }
  return events
}

function mapStreamEvent(event: ClaudeStreamEvent): ProviderEvent[] {
  if (event.type === 'content_block_start') {
    const block = event.content_block
    if (block.type === 'thinking') {
      return [{ type: 'thinking_start' }]
    }
    if (block.type === 'text') {
      return [{ type: 'text_delta', text: block.text }]
    }
  }
  if (event.type === 'content_block_delta') {
    const delta = event.delta
    switch (delta.type) {
      case 'text_delta':
        return [{ type: 'text_delta', text: delta.text }]
      case 'thinking_delta':
        return [{ type: 'thinking_delta', text: delta.thinking }]
      case 'signature_delta':
        return [{ type: 'thinking_signature', signature: delta.signature }]
    }
  }
  if (event.type === 'error') {
    return [{ type: 'error', message: event.error.message, code: event.error.type }]
  }
  return []
}

function parseControlRequest(
  message: Extract<ClaudeOutputMessage, { type: 'control_request' }>
): ClaudeControlRequest | null {
  if ('request' in message) {
    const inner = message.request.message
    // JSON-RPC notifications have no request ID and require no response.
    if (inner.id === undefined) {
      return null
    }
    return {
      protocol: 'sdk-mcp', outerRequestId: message.request_id,
      serverName: message.request.server_name, id: inner.id,
      toolUseId: inner.params?._meta?.['claudecode/toolUseId'],
      method: inner.method, params: inner.params,
    }
  }
  if (message.id === undefined) {
    return null
  }
  return { protocol: 'legacy', id: message.id, method: message.method, params: message.params }
}

/** The last iteration is the final API call, while top-level usage sums the turn. */
function mapResultUsage(usage: ClaudeUsage | undefined): TokenUsage {
  const counts = usage?.iterations?.at(-1) ?? usage
  return {
    inputTokens: counts?.input_tokens ?? counts?.inputTokens ?? 0,
    outputTokens: counts?.output_tokens ?? counts?.outputTokens ?? 0,
    cacheReadTokens: counts?.cache_read_input_tokens ?? counts?.cacheReadTokens ?? 0,
    cacheWriteTokens: counts?.cache_creation_input_tokens ?? counts?.cacheWriteTokens ?? 0,
  }
}

function resultErrorMessage(message: Extract<ClaudeOutputMessage, { type: 'result' }>): string {
  const parts = [message.result ?? '', ...message.errors ?? []]
    .map((text) => text.trim())
    .filter((text) => text.length > 0)
  return parts.join('\n') || 'Claude Code returned an error'
}

function classifyProviderError(message: string): string | null {
  const lower = message.toLowerCase()
  if (
    lower.includes('context_length_exceeded') ||
    lower.includes('context window') ||
    lower.includes('context length') ||
    lower.includes('maximum context') ||
    lower.includes('input is too long')
  ) {
    return 'context_length_exceeded'
  }
  if (
    lower.includes('rate_limit') ||
    lower.includes('rate limit') ||
    lower.includes('rate-limit') ||
    lower.includes('rate limited') ||
    lower.includes('too many requests') ||
    /\b429\b/.test(lower)
  ) {
    return 'rate_limit'
  }
  if (
    lower.includes('auth_expired') ||
    lower.includes('auth expired') ||
    lower.includes('authentication expired') ||
    lower.includes('auth failed') ||
    lower.includes('authentication failed') ||
    lower.includes('not logged in') ||
    lower.includes('login required') ||
    lower.includes('unauthorized')
  ) {
    return 'auth_expired'
  }
  return null
}

function stripMcpToolPrefix(name: string): string {
  const match = /^mcp__[^_]+__(.+)$/.exec(name)
  return match?.[1] ?? name
}
