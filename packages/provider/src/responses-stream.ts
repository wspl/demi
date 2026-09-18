/**
 * The Responses API stream, mapped to Demi's provider events. Shared by every
 * adapter that speaks it (the Codex backend and the OpenAI Responses
 * transport): the wire is the same, only the vendor's name in the error text
 * differs.
 *
 * The events arrive already decoded by `responses.ts`, so this module only
 * maps — no shape checks, no field guards.
 */
import { parseJsonOrString } from '@demicodes/utils'
import { zeroUsage } from '@demicodes/core'
import { normalizeErrorCode, upstreamDiagnostic } from './http'
import {
  decodeResponsesEvent,
  tokenUsageFromResponsesUsage,
  type ResponsesError,
  type ResponsesEvent,
  type ResponsesMessageItem,
  type ResponsesReasoningItem,
} from './responses'
import type { ServerSentEvent } from './sse'
import type { ProviderEvent } from './types'

/**
 * What one response is streaming, carried between events: a tool call's
 * arguments arrive as deltas keyed by item id, and the full text of a finished
 * item is only emitted when no delta carried it.
 */
interface ResponsesStreamState {
  /** Item id of the function call in flight, for deltas that omit `item_id`. */
  currentFunctionCallItemId: string | null
  /** Accumulated tool-call arguments, by item id. */
  functionArguments: Map<string, string>
  reasoningDeltaSeen: boolean
  textDeltaSeen: boolean
}

function newResponsesStreamState(): ResponsesStreamState {
  return {
    currentFunctionCallItemId: null,
    functionArguments: new Map(),
    reasoningDeltaSeen: false,
    textDeltaSeen: false,
  }
}

/**
 * Maps a decoded Responses stream — from SSE frames, a WebSocket, or any other
 * transport — to provider events. `vendorLabel` names the vendor in the errors
 * this reports (for example `Codex stream error`). When `signal` aborts, the
 * stream ends with an `abort` event.
 */
export async function* mapResponsesEvents(
  events: AsyncIterable<ResponsesEvent>,
  vendorLabel: string,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  const state = newResponsesStreamState()
  for await (const event of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    yield* mapResponsesEvent(event, vendorLabel, state)
  }
}

/**
 * Maps a Responses SSE body to provider events: each frame is decoded, event
 * types this adapter does not map are skipped, and a stream that ends without
 * `response.completed` still closes the turn with a zeroed `response` event.
 */
export async function* mapResponsesStream(
  frames: AsyncIterable<ServerSentEvent>,
  vendorLabel: string,
  signal?: AbortSignal,
): AsyncIterable<ProviderEvent> {
  let completed = false
  const events = async function* (): AsyncIterable<ResponsesEvent> {
    for await (const frame of frames) {
      const event = decodeResponsesFrame(frame.data)
      if (!event)
        continue
      if (event.type === 'response.completed')
        completed = true
      yield event
    }
  }

  yield* mapResponsesEvents(events(), vendorLabel, signal)
  // An aborted turn ends on its `abort` event; a finished one always reports
  // usage, even when the vendor closed the body without a completed response.
  if (!completed && !signal?.aborted)
    yield { type: 'response', usage: zeroUsage() }
}

/**
 * Decodes one SSE frame payload. Returns null for a payload that carries no
 * event — an empty keep-alive frame, or the `[DONE]` sentinel, which is not
 * JSON — and for an event type this adapter does not map; a mapped type with a
 * malformed payload throws.
 */
export function decodeResponsesFrame(data: string): ResponsesEvent | null {
  const payload = data.trim()
  if (!payload || payload === '[DONE]')
    return null
  return decodeResponsesEvent(JSON.parse(payload))
}

/** Maps one decoded event, advancing `state`. */
function* mapResponsesEvent(
  event: ResponsesEvent,
  vendorLabel: string,
  state: ResponsesStreamState,
): Iterable<ProviderEvent> {
  switch (event.type) {
    case 'response.output_item.added': {
      const item = event.item
      if (item?.type === 'reasoning')
        yield { type: 'thinking_start' }
      if (item?.type === 'function_call' && item.id) {
        state.currentFunctionCallItemId = item.id
        state.functionArguments.set(item.id, item.arguments ?? '')
      }
      return
    }
    case 'response.reasoning_summary_text.delta':
    case 'response.reasoning_text.delta':
      state.reasoningDeltaSeen = true
      yield { type: 'thinking_delta', text: event.delta }
      return
    case 'response.output_text.delta':
      state.textDeltaSeen = true
      yield { type: 'text_delta', text: event.delta }
      return
    case 'response.function_call_arguments.delta': {
      const itemId = event.item_id ?? state.currentFunctionCallItemId
      if (!itemId)
        return
      const collected = state.functionArguments.get(itemId) ?? ''
      state.functionArguments.set(itemId, `${collected}${event.delta}`)
      return
    }
    case 'response.function_call_arguments.done': {
      const itemId = event.item_id ?? state.currentFunctionCallItemId
      if (itemId)
        state.functionArguments.set(itemId, event.arguments)
      return
    }
    case 'response.output_item.done': {
      const item = event.item
      if (item?.type === 'reasoning') {
        // Emit the finished text only when no delta carried it (a
        // non-streaming response); otherwise it would arrive twice.
        if (!state.reasoningDeltaSeen) {
          const text = reasoningText(item)
          if (text)
            yield { type: 'thinking_delta', text }
        }
        yield { type: 'thinking_signature', signature: JSON.stringify(item) }
        state.reasoningDeltaSeen = false
        return
      }
      if (item?.type === 'message') {
        // Same rule as reasoning: the deltas already carried the whole text.
        if (!state.textDeltaSeen) {
          const text = messageText(item)
          if (text)
            yield { type: 'text_delta', text }
        }
        state.textDeltaSeen = false
        return
      }
      if (item?.type === 'function_call') {
        const itemId = item.id ?? event.item_id
        const callId = item.call_id ?? event.call_id
        if (itemId && callId && item.name) {
          const rawArguments = state.functionArguments.get(itemId)
            ?? item.arguments
            ?? '{}'
          yield {
            type: 'tool_call_requested',
            // The vendor identifies a call by both ids and needs both back on
            // the tool result, so Demi's single id carries the pair.
            toolUseId: `${callId}|${itemId}`,
            toolName: item.name,
            input: parseJsonOrString(rawArguments),
          }
        }
        if (itemId) {
          state.functionArguments.delete(itemId)
          if (state.currentFunctionCallItemId === itemId)
            state.currentFunctionCallItemId = null
        }
      }
      return
    }
    case 'response.completed':
      yield {
        type: 'response',
        usage: tokenUsageFromResponsesUsage(event.response?.usage)
      }
      return
    case 'response.failed': {
      const error = event.response?.error
      const message = error?.message ?? `${vendorLabel} response failed`
      const rawCode = error?.code ?? error?.type ?? null
      const providerRequestId = providerRequestIdFrom(error, message)
      const providerResponseId = event.response?.id
      const retryAfterMs = resetWaitMs(error)
      yield {
        type: 'error',
        message,
        code: normalizeErrorCode(rawCode, message),
        ...(retryAfterMs !== undefined ? { retryAfterMs } : {}),
        diagnostics: {
          source: 'stream',
          ...(rawCode ? { providerCode: rawCode } : {}),
          ...(providerRequestId ? { providerRequestId } : {}),
          ...(providerResponseId ? { providerResponseId } : {}),
          upstream: upstreamDiagnostic(event),
        },
      }
      return
    }
    case 'response.incomplete': {
      const reason = event.response?.incomplete_details?.reason ?? 'unknown'
      yield {
        type: 'error',
        message: `Incomplete ${vendorLabel} response returned, reason: ${reason}`,
        code: reason === 'max_output_tokens'
          ? 'context_length_exceeded'
          : 'incomplete',
      }
      return
    }
    case 'error': {
      // The SSE stream sends the failure flat as {message, code}; the Codex
      // WebSocket backend nests the same failure under `error`.
      const nested = event.error
      const message = event.message ?? nested?.message
        ?? `${vendorLabel} stream error`
      const rawCode = event.code ?? nested?.code ?? nested?.type ?? null
      const providerRequestId = providerRequestIdFrom(nested, message)
      const retryAfterMs = resetWaitMs(nested)
      yield {
        type: 'error',
        message,
        code: normalizeErrorCode(rawCode, message),
        ...(retryAfterMs !== undefined ? { retryAfterMs } : {}),
        diagnostics: {
          source: 'stream',
          ...(rawCode ? { providerCode: rawCode } : {}),
          ...(providerRequestId ? { providerRequestId } : {}),
          ...(event.status_code !== undefined ? { httpStatus: event.status_code } : {}),
          upstream: upstreamDiagnostic(event),
        },
      }
      return
    }
  }
}

/** A message item's text: its output text parts, refusals included. */
function messageText(item: ResponsesMessageItem): string {
  return (item.content ?? [])
    .map((part) => {
      if (part?.type === 'output_text')
        return part.text
      if (part?.type === 'refusal')
        return part.refusal
      return ''
    })
    .join('')
}

/** A reasoning item's text: its summary, or its content when it has none. */
function reasoningText(item: ResponsesReasoningItem): string {
  const summary = item.summary?.map((part) => part.text).join('\n\n') ?? ''
  const content = item.content?.map((part) => part.text).join('\n\n') ?? ''
  return summary || content
}

/**
 * The vendor's request id for a failure: the field when the error carries one,
 * otherwise the id OpenAI embeds in the message it asks users to quote.
 */
function providerRequestIdFrom(
  error: ResponsesError | undefined,
  message: string,
): string | null {
  const explicit = error?.request_id ?? error?.requestId
  if (explicit)
    return explicit
  return message.match(/request ID ([A-Za-z0-9-]+)/i)?.[1] ?? null
}

/** How long until a vendor's limit lifts, from the fields its error names; undefined when it names none. */
function resetWaitMs(error: ResponsesError | undefined): number | undefined {
  if (error?.resets_in_seconds !== undefined)
    return error.resets_in_seconds * 1000
  if (error?.resets_at !== undefined)
    return Math.max(0, error.resets_at * 1000 - Date.now())
  return undefined
}
