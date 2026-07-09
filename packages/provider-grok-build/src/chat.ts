import { isRecord, numberOrZero, parseJsonObject, parseJsonOrString, stringOrNull } from '@demicodes/utils'
import { Buffer } from 'node:buffer'
import { zeroUsage } from '@demicodes/core'
import type { ToolResultContentBlock, UserContentBlock } from '@demicodes/core'
import { normalizeErrorCode, type InferenceItem, type InferenceRequest, type ProviderEvent, type ToolDefinition } from '@demicodes/provider'

export interface GrokChatCompletionsRequestBody {
  model: string
  messages: GrokChatMessage[]
  stream: true
  tools?: GrokChatTool[]
  tool_choice?: 'auto'
  reasoning_effort?: string
  stream_options?: Record<string, unknown>
  [key: string]: unknown
}

export type GrokChatMessage =
  | { role: 'system'; content: string }
  | { role: 'user'; content: string | GrokUserContentPart[] }
  | { role: 'assistant'; content: string | null; tool_calls?: GrokChatToolCall[] }
  | { role: 'tool'; tool_call_id: string; content: string }

export type GrokUserContentPart =
  | { type: 'text'; text: string }
  | { type: 'image_url'; image_url: { url: string; detail?: 'auto' | 'low' | 'high' } }

export interface GrokChatTool {
  type: 'function'
  function: {
    name: string
    description: string
    parameters: Record<string, unknown>
  }
}

export interface GrokChatToolCall {
  id: string
  type: 'function'
  function: {
    name: string
    arguments: string
  }
}

export interface ServerSentEvent {
  event: string | null
  data: string[]
}

export function buildGrokChatCompletionsBody(request: InferenceRequest): GrokChatCompletionsRequestBody {
  const body: GrokChatCompletionsRequestBody = {
    model: request.modelId,
    messages: inferenceItemsToMessages(request.systemPrompt, request.items),
    stream: true,
    stream_options: { include_usage: true },
  }
  if (request.tools.length > 0) {
    body.tools = request.tools.map(toolToGrokTool)
    body.tool_choice = 'auto'
  }
  const reasoningEffort = thinkingToReasoningEffort(request)
  if (reasoningEffort) body.reasoning_effort = reasoningEffort
  return body
}

export async function* mapGrokChatCompletionStream(
  events: AsyncIterable<ServerSentEvent>,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const toolCalls = new Map<number, MutableToolCall>()
  let thinkingStarted = false
  let usage = zeroUsage()

  for await (const event of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    for (const data of event.data) {
      if (data === '[DONE]') {
        yield* flushToolCalls(toolCalls)
        yield { type: 'response', usage }
        return
      }
      const chunk = parseJsonObject(data)
      if (!chunk) continue
      const error = isRecord(chunk.error) ? chunk.error : null
      if (error) {
        const message = stringOrNull(error.message) ?? 'Grok Build stream error'
        yield {
          type: 'error',
          message,
          code: normalizeErrorCode(stringOrNull(error.code) ?? stringOrNull(error.type), message),
        }
        return
      }
      if (isRecord(chunk.usage)) usage = grokUsage(chunk.usage)
      const choices = Array.isArray(chunk.choices) ? chunk.choices : []
      for (const choice of choices) {
        if (!isRecord(choice)) continue
        const delta = isRecord(choice.delta) ? choice.delta : null
        if (delta) {
          const reasoning = stringOrNull(delta.reasoning_content)
          if (reasoning) {
            if (!thinkingStarted) {
              thinkingStarted = true
              yield { type: 'thinking_start' }
            }
            yield { type: 'thinking_delta', text: reasoning }
          }
          const content = stringOrNull(delta.content)
          if (content) yield { type: 'text_delta', text: content }
          if (Array.isArray(delta.tool_calls)) collectToolCalls(delta.tool_calls, toolCalls)
        }
        if (choice.finish_reason === 'tool_calls') yield* flushToolCalls(toolCalls)
      }
    }
  }

  yield* flushToolCalls(toolCalls)
  yield { type: 'response', usage }
}

export async function* readServerSentEvents(
  body: ReadableStream<Uint8Array> | null,
  signal?: AbortSignal,
): AsyncIterable<ServerSentEvent> {
  if (!body) return
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  let eventName: string | null = null
  let data: string[] = []

  const flush = function* (): Iterable<ServerSentEvent> {
    if (data.length === 0) return
    yield { event: eventName, data }
    eventName = null
    data = []
  }

  try {
    while (true) {
      if (signal?.aborted) return
      const { value, done } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      let newline = buffer.indexOf('\n')
      while (newline !== -1) {
        const raw = buffer.slice(0, newline)
        buffer = buffer.slice(newline + 1)
        const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw
        if (line === '') {
          yield* flush()
        } else if (line.startsWith('event:')) {
          eventName = line.slice('event:'.length).trim()
        } else if (line.startsWith('data:')) {
          data.push(line.slice('data:'.length).trimStart())
        }
        newline = buffer.indexOf('\n')
      }
    }
    buffer += decoder.decode()
    if (buffer) {
      const line = buffer.endsWith('\r') ? buffer.slice(0, -1) : buffer
      if (line.startsWith('data:')) data.push(line.slice('data:'.length).trimStart())
      else if (line.startsWith('event:')) eventName = line.slice('event:'.length).trim()
    }
    yield* flush()
  } finally {
    reader.releaseLock()
  }
}

interface MutableToolCall {
  id: string
  name: string
  arguments: string
}

function inferenceItemsToMessages(systemPrompt: string, items: InferenceItem[]): GrokChatMessage[] {
  const messages: GrokChatMessage[] = []
  let assistant: { content: string; toolCalls: GrokChatToolCall[] } | null = null

  const flushAssistant = () => {
    if (!assistant) return
    messages.push({
      role: 'assistant',
      content: assistant.content || null,
      ...(assistant.toolCalls.length > 0 ? { tool_calls: assistant.toolCalls } : {}),
    })
    assistant = null
  }

  if (systemPrompt.trim()) messages.push({ role: 'system', content: systemPrompt })

  for (const item of items) {
    switch (item.type) {
      case 'user_message':
      case 'user_steer':
        flushAssistant()
        messages.push({ role: 'user', content: userContentToParts(item.content) })
        break
      case 'assistant_text':
        assistant ??= { content: '', toolCalls: [] }
        assistant.content += item.text
        break
      case 'tool_use':
        assistant ??= { content: '', toolCalls: [] }
        assistant.toolCalls.push({
          id: item.toolUseId,
          type: 'function',
          function: { name: item.toolName, arguments: stringifyToolArguments(item.input) },
        })
        break
      case 'tool_result':
        flushAssistant()
        messages.push({ role: 'tool', tool_call_id: item.toolUseId, content: toolResultContentToText(item.output) })
        break
      case 'assistant_thinking':
      case 'assistant_redacted_thinking':
        break
    }
  }
  flushAssistant()
  return messages
}

function userContentToParts(content: UserContentBlock[]): string | GrokUserContentPart[] {
  const parts: GrokUserContentPart[] = []
  for (const block of content) {
    if (block.type === 'text') parts.push({ type: 'text', text: block.text })
    else if (block.type === 'reference') parts.push({ type: 'text', text: block.reference })
    else if (block.type === 'document') {
      parts.push({ type: 'text', text: `[document:${block.source.fileName} ${block.source.mediaType}]` })
    } else if (block.source.type === 'url') {
      parts.push({ type: 'image_url', image_url: { url: block.source.url, detail: 'auto' } })
    } else {
      parts.push({
        type: 'image_url',
        image_url: {
          url: `data:${block.source.mediaType};base64,${Buffer.from(block.source.data).toString('base64')}`,
          detail: 'auto',
        },
      })
    }
  }
  if (parts.every((part) => part.type === 'text')) return parts.map((part) => part.text).join('\n')
  return parts
}

function toolResultContentToText(output: ToolResultContentBlock[]): string {
  return output.map((block) => (block.type === 'text' ? block.text : `[image:${block.source.mediaType}]`)).join('\n')
}

function toolToGrokTool(tool: ToolDefinition): GrokChatTool {
  return {
    type: 'function',
    function: {
      name: tool.name,
      description: tool.description,
      parameters: tool.inputSchema,
    },
  }
}

function collectToolCalls(values: unknown[], toolCalls: Map<number, MutableToolCall>): void {
  for (const value of values) {
    if (!isRecord(value)) continue
    const index = typeof value.index === 'number' ? value.index : toolCalls.size
    const existing = toolCalls.get(index) ?? { id: '', name: '', arguments: '' }
    const fn = isRecord(value.function) ? value.function : null
    const id = stringOrNull(value.id)
    if (id) existing.id = id
    const name = stringOrNull(fn?.name)
    if (name) existing.name = name
    const delta = stringOrNull(fn?.arguments)
    if (delta) existing.arguments += delta
    toolCalls.set(index, existing)
  }
}

function* flushToolCalls(toolCalls: Map<number, MutableToolCall>): Iterable<ProviderEvent> {
  for (const [index, call] of [...toolCalls.entries()].sort(([a], [b]) => a - b)) {
    if (!call.name) continue
    yield {
      type: 'tool_call_requested',
      toolUseId: call.id || `tool_call_${index}`,
      toolName: call.name,
      input: parseJsonOrString(call.arguments || '{}'),
    }
  }
  toolCalls.clear()
}

function thinkingToReasoningEffort(request: InferenceRequest): string | undefined {
  const thinking = request.thinking
  if (!thinking || thinking.type === 'disabled' || thinking.type === 'budget') return undefined
  return thinking.effort
}

function stringifyToolArguments(input: unknown): string {
  return typeof input === 'string' ? input : JSON.stringify(input ?? {})
}

function grokUsage(usage: Record<string, unknown>) {
  const inputTokens = numberOrZero(usage.prompt_tokens)
  const outputTokens = numberOrZero(usage.completion_tokens)
  const promptDetails = isRecord(usage.prompt_tokens_details) ? usage.prompt_tokens_details : null
  const cachedTokens = promptDetails ? numberOrZero(promptDetails.cached_tokens) : 0
  return {
    inputTokens: Math.max(0, inputTokens - cachedTokens),
    outputTokens,
    cacheReadTokens: cachedTokens,
    cacheWriteTokens: 0,
  }
}
