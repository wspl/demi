/**
 * The Chat Completions stream, mapped to Demi's provider events. Shared by
 * every OpenAI-compatible adapter: the wire is the same, only the vendor's
 * name in the error text differs.
 */
import { parseJsonOrString } from '@demicodes/utils'
import { zeroUsage } from '@demicodes/core'
import {
  decodeChatCompletionChunk,
  tokenUsageFromChatCompletionsUsage,
  type ChatCompletionToolCallDelta,
} from './chat-completions'
import { normalizeErrorCode, withRetryWait } from './http'
import type { ServerSentEvent } from './sse'
import type { ProviderEvent, ProviderFailureReader } from './types'

/** A tool call being assembled from the increments of one `index`. */
interface CollectedToolCall {
  id: string
  name: string
  arguments: string
}

/**
 * Maps a Chat Completions SSE body to provider events. `vendorLabel` names the
 * vendor in the errors this reports (for example `Grok Build stream error`),
 * and `readFailure` is the provider's reader, which sets a failure's retry
 * wait. When `signal` aborts, the stream ends with an `abort` event.
 *
 * A turn ends with one `response` event, whether the vendor closed it with the
 * `[DONE]` sentinel or just ended the body; the tool calls collected so far are
 * emitted first. A vendor error ends the turn as an error instead.
 */
export async function* mapChatCompletionsStream(
  frames: AsyncIterable<ServerSentEvent>,
  vendorLabel: string,
  readFailure: ProviderFailureReader,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const toolCalls = new Map<number, CollectedToolCall>()
  let thinkingStarted = false
  let usage = zeroUsage()

  for await (const frame of frames) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    if (frame.data === '[DONE]') {
      yield* flushToolCalls(toolCalls)
      yield { type: 'response', usage }
      return
    }
    const chunk = decodeChatCompletionChunk(JSON.parse(frame.data))
    if (chunk.error) {
      const message = chunk.error.message ?? `${vendorLabel} stream error`
      const rawCode = chunk.error.code ?? chunk.error.type ?? null
      yield withRetryWait({
        type: 'error',
        message,
        code: normalizeErrorCode(rawCode, message),
        diagnostics: { source: 'stream', upstream: frame.data },
      }, readFailure)
      return
    }
    // With `stream_options.include_usage` only the final chunk carries counts.
    if (chunk.usage)
      usage = tokenUsageFromChatCompletionsUsage(chunk.usage)

    for (const choice of chunk.choices ?? []) {
      const reasoning = choice.delta?.reasoning_content
      if (reasoning) {
        if (!thinkingStarted) {
          thinkingStarted = true
          yield { type: 'thinking_start' }
        }
        yield { type: 'thinking_delta', text: reasoning }
      }
      const content = choice.delta?.content
      if (content)
        yield { type: 'text_delta', text: content }
      if (choice.delta?.tool_calls)
        collectToolCalls(choice.delta.tool_calls, toolCalls)
      if (choice.finish_reason === 'tool_calls')
        yield* flushToolCalls(toolCalls)
    }
  }

  yield* flushToolCalls(toolCalls)
  yield { type: 'response', usage }
}

/** Folds one chunk's tool-call increments into the calls being assembled. */
function collectToolCalls(
  deltas: ChatCompletionToolCallDelta[],
  toolCalls: Map<number, CollectedToolCall>,
): void {
  for (const delta of deltas) {
    // A vendor that omits `index` sends one call at a time, in order.
    const index = delta.index ?? toolCalls.size
    const call = toolCalls.get(index) ?? { id: '', name: '', arguments: '' }
    if (delta.id)
      call.id = delta.id
    if (delta.function?.name)
      call.name = delta.function.name
    if (delta.function?.arguments)
      call.arguments += delta.function.arguments
    toolCalls.set(index, call)
  }
}

/** Emits the assembled tool calls in index order and clears them. */
function* flushToolCalls(
  toolCalls: Map<number, CollectedToolCall>
): Iterable<ProviderEvent> {
  const byIndex = [...toolCalls.entries()].sort(([a], [b]) => a - b)
  for (const [index, call] of byIndex) {
    if (!call.name)
      continue
    yield {
      type: 'tool_call_requested',
      toolUseId: call.id || `tool_call_${index}`,
      toolName: call.name,
      // A vendor that streams malformed JSON arguments still names a tool; the
      // agent reports the bad input rather than losing the call.
      input: parseJsonOrString(call.arguments || '{}'),
    }
  }
  toolCalls.clear()
}
