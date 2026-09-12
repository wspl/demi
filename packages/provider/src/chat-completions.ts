import { zeroUsage, type TokenUsage } from '@demicodes/core'
import { parseJsonOrString } from '@demicodes/utils'
import { chatCompletionChunkSchema, type ChatToolDelta, type ChatUsage } from './chat-completions-wire'
import { normalizeErrorCode } from './http'
import { parseProviderJson, ProviderDataError } from './validation'
import type { ServerSentEvent } from './sse'
import type { ProviderEvent } from './types'

export interface ChatCompletionStreamOptions {
  source?: string
  signal?: AbortSignal
}

type ToolCallState = Pick<ChatToolDelta, 'id'>
  & Pick<NonNullable<ChatToolDelta['function']>, 'name' | 'arguments'>
interface ChoiceState {
  status: 'running' | 'finished'
  thinkingStarted: boolean
  toolCalls: Map<number, ToolCallState>
}

/** Maps the Chat Completions wire used by OpenAI-compatible and Grok endpoints. */
export async function* mapChatCompletionStream(
  events: AsyncIterable<ServerSentEvent>,
  { source = 'Chat Completions', signal }: ChatCompletionStreamOptions = {},
): AsyncIterable<ProviderEvent> {
  const choices = new Map<number, ChoiceState>()
  const completedToolIds = new Set<string>()
  let usage = zeroUsage()
  for await (const frame of events) {
    if (signal?.aborted) {
      yield { type: 'abort' }
      return
    }
    if (frame.data === '[DONE]') {
      for (const choice of choices.values()) {
        yield* finishToolCalls(choice.toolCalls, completedToolIds, source)
      }
      yield { type: 'response', usage }
      return
    }
    const chunk = parseProviderJson(chatCompletionChunkSchema, frame.data, `${source} event`)
    if (chunk.error) {
      const message = chunk.error.message ?? `${source} stream error`
      yield {
        type: 'error', message,
        code: normalizeErrorCode(chunk.error.code ?? chunk.error.type ?? null, message),
      }
      return
    }
    if (chunk.usage)
      usage = chatUsage(chunk.usage)
    for (const [position, choice] of (chunk.choices ?? []).entries()) {
      const index = choice.index ?? position
      const state = choices.get(index) ?? {
        status: 'running', thinkingStarted: false, toolCalls: new Map<number, ToolCallState>(),
      }
      choices.set(index, state)
      const delta = choice.delta
      if (state.status === 'finished' && (delta?.content || delta?.reasoning_content || delta?.tool_calls?.length))
        throw new ProviderDataError(source, 'delta arrived after choice completion')
      const finishReason = choice.finish_reason
      if (finishReason && finishReason !== 'stop' && finishReason !== 'tool_calls') {
        yield {
          type: 'error', message: `${source} generation stopped: ${finishReason}`,
          code: finishReason === 'length' ? 'context_length_exceeded' : 'incomplete',
        }
        return
      }
      if (delta?.reasoning_content) {
        if (!state.thinkingStarted) {
          state.thinkingStarted = true
          yield { type: 'thinking_start' }
        }
        yield { type: 'thinking_delta', text: delta.reasoning_content }
      }
      if (delta?.content)
        yield { type: 'text_delta', text: delta.content }
      for (const toolDelta of delta?.tool_calls ?? []) {
        appendToolDelta(state.toolCalls, toolDelta, source)
      }
      if (finishReason === 'tool_calls')
        yield* finishToolCalls(state.toolCalls, completedToolIds, source)
      if (finishReason)
        state.status = 'finished'
    }
  }
  signal?.throwIfAborted()
  throw new ProviderDataError(source, 'stream ended before [DONE]')
}

function appendToolDelta(
  calls: Map<number, ToolCallState>,
  delta: ChatToolDelta,
  source: string,
): void {
  const call = calls.get(delta.index) ?? {}
  if (delta.id !== undefined) {
    if (call.id !== undefined && call.id !== delta.id)
      throw new ProviderDataError(source, 'tool identity changed during streaming')
    call.id = delta.id
  }
  if (delta.function?.name !== undefined) {
    if (call.name !== undefined && call.name !== delta.function.name)
      throw new ProviderDataError(source, 'tool name changed during streaming')
    call.name = delta.function.name
  }
  if (delta.function?.arguments !== undefined)
    call.arguments = (call.arguments ?? '') + delta.function.arguments
  calls.set(delta.index, call)
}

function finishToolCalls(
  calls: Map<number, ToolCallState>,
  completedIds: Set<string>,
  source: string,
): ProviderEvent[] {
  const events: ProviderEvent[] = []
  const ids = new Set<string>()
  for (const [, call] of [...calls.entries()].sort(([a], [b]) => a - b)) {
    if (call.id === undefined || call.name === undefined || call.arguments === undefined)
      throw new ProviderDataError(source, 'completed tool call is missing identity, name or arguments')
    if (completedIds.has(call.id) || ids.has(call.id))
      throw new ProviderDataError(source, 'duplicate tool call identity')
    ids.add(call.id)
    events.push({
      type: 'tool_call_requested', toolUseId: call.id, toolName: call.name,
      input: parseJsonOrString(call.arguments),
    })
  }
  for (const id of ids)
    completedIds.add(id)
  calls.clear()
  return events
}

function chatUsage(usage: ChatUsage): TokenUsage {
  const cached = usage.prompt_tokens_details?.cached_tokens ?? 0
  return {
    inputTokens: (usage.prompt_tokens ?? 0) - cached,
    outputTokens: usage.completion_tokens ?? 0,
    cacheReadTokens: cached,
    cacheWriteTokens: 0,
  }
}
