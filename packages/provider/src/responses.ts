import { parseJsonOrString } from '@demicodes/utils'
import type { ResponsesStreamEvent } from './responses-wire'
import type { ProviderEvent } from './types'

export interface ResponsesContentState {
  reasoningDeltaSeen: boolean
  textDeltaSeen: boolean
}

export function createResponsesContentState(): ResponsesContentState {
  return { reasoningDeltaSeen: false, textDeltaSeen: false }
}

/** Maps shared content events; adapters map their terminal/error diagnostics. */
export function* mapResponsesContentEvent(
  event: ResponsesStreamEvent,
  state: ResponsesContentState,
): Iterable<ProviderEvent> {
  switch (event.type) {
    case 'response.output_item.added':
      if (event.item.type === 'reasoning')
        yield { type: 'thinking_start' }
      return
    case 'response.reasoning_summary_text.delta':
    case 'response.reasoning_text.delta':
      state.reasoningDeltaSeen = true
      yield { type: 'thinking_delta', text: event.delta }
      return
    case 'response.output_text.delta':
      state.textDeltaSeen = true
      yield { type: 'text_delta', text: event.delta }
      return
    case 'response.function_call_arguments.delta':
    case 'response.function_call_arguments.done':
      // The completed output item carries authoritative full arguments.
      return
    case 'response.output_item.done': {
      const item = event.item
      if (item.type === 'reasoning') {
        if (!state.reasoningDeltaSeen) {
          const summary = item.summary?.map((part) => part.text).join('\n\n') ?? ''
          const content = item.content?.map((part) => part.text).join('\n\n') ?? ''
          const text = summary || content
          if (text)
            yield { type: 'thinking_delta', text }
        }
        yield { type: 'thinking_signature', signature: JSON.stringify(item) }
        state.reasoningDeltaSeen = false
      } else if (item.type === 'message') {
        // Completed text is the fallback only when no deltas were delivered.
        if (!state.textDeltaSeen) {
          const text = item.content.map((part) => part.type === 'output_text' ? part.text : part.refusal).join('')
          if (text)
            yield { type: 'text_delta', text }
        }
        state.textDeltaSeen = false
      } else if (item.type === 'function_call') {
        yield {
          type: 'tool_call_requested',
          toolUseId: `${item.call_id}|${item.id}`,
          toolName: item.name,
          input: parseJsonOrString(item.arguments),
        }
      }
      return
    }
  }
}
