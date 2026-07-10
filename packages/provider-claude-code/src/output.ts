import { isRecord, stringOrNull } from '@demicodes/utils'
import type { TokenUsage } from '@demicodes/core'
import type { ProviderEvent } from '@demicodes/provider'

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
  params?: unknown
}

export interface ClaudeOutputMapOptions {
  ignoreAssistantContent?: boolean
  ignoreAssistantToolUse?: boolean
  /** Per-request usage captured from the latest assistant message. */
  responseUsage?: TokenUsage | null
}

export function mapClaudeStdoutMessage(message: unknown, options: ClaudeOutputMapOptions = {}): OutputMapping {
  const events: ProviderEvent[] = []
  if (!isRecord(message)) return { events, terminal: false }

  if (message.type === 'assistant' && isRecord(message.message) && !options.ignoreAssistantContent) {
    events.push(...mapContentArray((message.message as { content?: unknown }).content, options))
  }

  if (message.type === 'stream_event' && isRecord(message.event)) {
    events.push(...mapStreamEvent(message.event))
  }

  if (message.type === 'control_request') {
    const request = parseControlRequest(message)
    if (request) return { events, controlRequest: request, terminal: false }
  }

  if (message.type === 'result') {
    if (message.is_error === true) {
      const errorMessage = resultErrorMessage(message)
      events.push({ type: 'error', message: errorMessage, code: classifyProviderError(errorMessage) })
    }
    events.push({ type: 'response', usage: options.responseUsage ?? mapUsage(message.usage) })
    return { events, terminal: true }
  }

  if (message.type === 'error') {
    const errorMessage = String(message.message ?? 'Claude Code error')
    events.push({ type: 'error', message: errorMessage, code: stringOrNull(message.code) ?? classifyProviderError(errorMessage) })
  }

  return { events, terminal: false }
}

/** Claude Code result usage is cumulative across a tool-heavy turn; assistant usage is per API call. */
export function claudeAssistantUsage(message: unknown): TokenUsage | null {
  if (!isRecord(message) || message.type !== 'assistant' || !isRecord(message.message)) return null
  return isRecord(message.message.usage) ? mapUsage(message.message.usage) : null
}

export function controlRequestToToolCall(request: ClaudeControlRequest): ProviderEvent | null {
  if (request.method !== 'tools/call') return null
  if (!isRecord(request.params)) return null
  const name = typeof request.params.name === 'string' ? request.params.name : null
  if (!name) return null
  return {
    type: 'tool_call_requested',
    toolUseId: request.toolUseId ?? String(request.id),
    toolName: stripMcpToolPrefix(name),
    input: request.params.arguments ?? request.params.input ?? {},
  }
}

function mapContentArray(content: unknown, options: ClaudeOutputMapOptions): ProviderEvent[] {
  if (!Array.isArray(content)) return []
  const events: ProviderEvent[] = []
  for (const block of content) {
    if (!isRecord(block)) continue
    if (block.type === 'text') events.push({ type: 'text_delta', text: String(block.text ?? '') })
    else if (block.type === 'thinking') {
      events.push({ type: 'thinking_start' })
      events.push({ type: 'thinking_delta', text: String(block.thinking ?? block.text ?? '') })
      if (typeof block.signature === 'string') events.push({ type: 'thinking_signature', signature: block.signature })
    } else if (block.type === 'redacted_thinking') {
      events.push({ type: 'redacted_thinking', data: String(block.data ?? '') })
    } else if (block.type === 'tool_use' && !options.ignoreAssistantToolUse) {
      events.push(mapToolUseBlock(block))
    }
  }
  return events
}

function mapToolUseBlock(block: Record<string, unknown>): ProviderEvent {
  const id = typeof block.id === 'string' || typeof block.id === 'number' ? String(block.id) : null
  const name = typeof block.name === 'string' && block.name.length > 0 ? block.name : null
  if (!id || !name) {
    return { type: 'error', message: 'Invalid tool_use block from Claude Code', code: null }
  }
  return {
    type: 'tool_call_requested',
    toolUseId: id,
    toolName: stripMcpToolPrefix(name),
    input: block.input ?? {},
  }
}

function mapStreamEvent(event: Record<string, unknown>): ProviderEvent[] {
  if (event.type === 'content_block_start' && isRecord(event.content_block)) {
    const block = event.content_block
    if (block.type === 'thinking') return [{ type: 'thinking_start' }]
    if (block.type === 'text' && typeof block.text === 'string') return [{ type: 'text_delta', text: block.text }]
  }

  if (event.type === 'content_block_delta' && isRecord(event.delta)) {
    const delta = event.delta
    if (delta.type === 'text_delta') return [{ type: 'text_delta', text: String(delta.text ?? '') }]
    if (delta.type === 'thinking_delta') return [{ type: 'thinking_delta', text: String(delta.thinking ?? '') }]
    if (delta.type === 'signature_delta') return [{ type: 'thinking_signature', signature: String(delta.signature ?? '') }]
  }

  return []
}

function parseControlRequest(message: Record<string, unknown>): ClaudeControlRequest | null {
  if (typeof message.request_id === 'string' && isRecord(message.request)) {
    const request = message.request
    if (request.subtype !== 'mcp_message') return null
    if (request.server_name !== 'main') return null
    if (!isRecord(request.message)) return null
    const inner = request.message
    const id = typeof inner.id === 'string' || typeof inner.id === 'number' ? inner.id : 0
    const method = typeof inner.method === 'string' ? inner.method : undefined
    if (!method) return null
    return {
      protocol: 'sdk-mcp',
      outerRequestId: message.request_id,
      serverName: request.server_name,
      id,
      method,
      params: inner.params,
    }
  }

  const id = typeof message.id === 'string' || typeof message.id === 'number' ? message.id : undefined
  const method = typeof message.method === 'string' ? message.method : undefined
  if (id === undefined || !method) return null
  return { protocol: 'legacy', id, method, params: message.params }
}

function mapUsage(usage: unknown): TokenUsage {
  if (!isRecord(usage)) {
    return { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }
  }
  return {
    inputTokens: numberValue(usage.input_tokens ?? usage.inputTokens),
    outputTokens: numberValue(usage.output_tokens ?? usage.outputTokens),
    cacheReadTokens: numberValue(usage.cache_read_input_tokens ?? usage.cacheReadTokens),
    cacheWriteTokens: numberValue(usage.cache_creation_input_tokens ?? usage.cacheWriteTokens),
  }
}

function numberValue(value: unknown): number {
  return typeof value === 'number' ? value : 0
}

function resultErrorMessage(message: Record<string, unknown>): string {
  const parts: string[] = []
  if (typeof message.result === 'string' && message.result.trim()) parts.push(message.result.trim())
  if (Array.isArray(message.errors)) {
    for (const error of message.errors) {
      const text = String(error).trim()
      if (text) parts.push(text)
    }
  }
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
