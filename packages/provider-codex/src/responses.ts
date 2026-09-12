import { attachmentTag } from '@demicodes/core'
import {
  responsesReasoningItemSchema,
  type ResponsesStreamEvent,
  type ResponsesReasoningItem,
  type ResponsesFailed,
} from '@demicodes/provider'
import { parseProviderData, responsesUsage, createResponsesContentState, mapResponsesContentEvent, type ResponsesContentState } from '@demicodes/provider'
import {
  isRecord,
  shortHash,
} from '@demicodes/utils'
import { Buffer } from 'node:buffer'
import type {
  ToolResultContentBlock,
  UserContentBlock
} from '@demicodes/core'
import {
  clampPromptCacheKey,
  normalizeErrorCode,
  type InferenceItem,
  type InferenceRequest,
  type ProviderEvent,
  type ToolDefinition
} from '@demicodes/provider'

export interface CodexResponsesRequestBody {
  model: string
  instructions: string
  input: CodexResponseInputItem[]
  tools: CodexResponseTool[]
  tool_choice: 'auto'
  parallel_tool_calls: boolean
  reasoning?: {
    effort?: string;
    summary?: string
  }
  service_tier?: string
  store: boolean
  stream: boolean
  include: string[]
  prompt_cache_key: string
  text?: { verbosity?: 'low' | 'medium' | 'high' }
  [key: string]: unknown
}

export type CodexResponseInputItem =
  | {
      type: 'message';
      role: 'assistant';
      content: Array<{
        type: 'output_text';
        text: string;
        annotations: unknown[]
      }>;
      id?: string;
      status?: 'completed';
      phase?: string
    }
  | {
      role: 'user';
      content: CodexUserContent[]
    }
  | {
      type: 'reasoning';
      id?: string;
      summary?: Array<{
        type?: string;
        text: string
      }>;
      content?: Array<{
        type?: string;
        text: string
      }>;
      encrypted_content?: string | null
    }
  | {
      type: 'function_call';
      id?: string;
      call_id: string;
      name: string;
      arguments: string
    }
  | {
      type: 'function_call_output';
      call_id: string;
      output: string | CodexUserContent[]
    }

export type CodexUserContent =
  | {
      type: 'input_text';
      text: string
    }
  | {
      type: 'input_image';
      image_url: string;
      detail?: 'auto' | 'low' | 'high'
    }
  | {
      type: 'input_file';
      filename: string;
      file_data: string
    }

export interface CodexResponseTool {
  type: 'function'
  name: string
  description: string
  parameters: Record<string, unknown>
  strict: null
}

export function buildCodexResponsesRequestBody(
  request: InferenceRequest
): CodexResponsesRequestBody {
  const body: CodexResponsesRequestBody = {
    model: request.modelId,
    instructions: request.systemPrompt,
    input: request.items.flatMap((item, index) => inferenceItemToResponsesInput(
      item,
      index
    )),
    tools: request.tools.map(toolToResponsesTool),
    tool_choice: 'auto',
    parallel_tool_calls: true,
    store: false,
    stream: true,
    include: ['reasoning.encrypted_content'],
    prompt_cache_key: clampPromptCacheKey(request.sessionId),
    text: { verbosity: 'low' },
  }
  const reasoning = thinkingToReasoning(request.thinking)
  if (reasoning)
    body.reasoning = reasoning
  if (request.serviceTierId)
    body.service_tier = request.serviceTierId
  return body
}

export async function* mapCodexResponseEvents(
  events: AsyncIterable<ResponsesStreamEvent>
): AsyncIterable<ProviderEvent> {
  const state = createResponsesContentState()

  for await (const event of events) {
    yield* mapCodexResponseEvent(event, state)
  }
}

export function* mapCodexResponseEvent(
  event: ResponsesStreamEvent,
  state: ResponsesContentState = createResponsesContentState()
): Iterable<ProviderEvent> {
  yield* mapResponsesContentEvent(event, state)
  switch (event.type) {
    case 'response.completed':
      yield { type: 'response', usage: responsesUsage(event.response) }
      return
    case 'response.failed':
      yield errorEventFromFailedResponse(event.response)
      return
    case 'response.incomplete':
      yield {
        type: 'error',
        message: `Incomplete response returned, reason: ${incompleteReason(event.response)}`,
        code: incompleteReason(event.response) === 'max_output_tokens'
          ? 'context_length_exceeded'
          : 'incomplete',
      }
      return
    case 'error': {
      // Over the WebSocket transport the backend nests request failures as
      // {type:'error', error:{type, message, code}, status} instead of the
      // flat SSE shape, so read both before falling back to the generic text.
      const nested = event.error
      const message = event.message ?? nested?.message
        ?? 'Codex stream error'
      const rawCode = event.code ?? nested?.code
        ?? nested?.type ?? null
      const providerRequestId = providerRequestIdFrom(nested, message)
      yield {
        type: 'error',
        message,
        code: normalizeErrorCode(rawCode, message),
        diagnostics: {
          source: 'stream',
          ...(rawCode ? { providerCode: rawCode } : {}),
          ...(providerRequestId ? { providerRequestId } : {}),
        },
      }
      return
    }
  }
}

export function splitCodexToolUseId(
  toolUseId: string
): {
  callId: string;
  itemId: string | undefined
} {
  const [callId, itemId] = toolUseId.split('|', 2)
  return { callId, itemId }
}

function inferenceItemToResponsesInput(
  item: InferenceItem,
  index: number
): CodexResponseInputItem[] {
  switch (item.type) {
    case 'user_message':
    case 'user_steer':
      return [{ role: 'user', content: userContentToResponses(item.content) }]
    case 'assistant_text':
      return [
        {
          type: 'message',
          role: 'assistant',
          content: [{ type: 'output_text', text: item.text, annotations: [] }],
          id: `msg_${shortHash(`${index}:${item.modelId}:${item.text}`)}`,
          status: 'completed',
        },
      ]
    case 'assistant_thinking': {
      const reasoning = parseReasoningSignature(item.signature)
      return reasoning ? [reasoning] : []
    }
    case 'assistant_redacted_thinking':
      return []
    case 'tool_use': {
      const { callId, itemId } = splitCodexToolUseId(item.toolUseId)
      return [
        {
          type: 'function_call',
          id: itemId,
          call_id: callId,
          name: item.toolName,
          arguments: stringifyArguments(item.input),
        },
      ]
    }
    case 'tool_result': {
      const { callId } = splitCodexToolUseId(item.toolUseId)
      return [{
        type: 'function_call_output',
        call_id: callId,
        output: toolResultToResponsesOutput(item.output)
      }]
    }
  }
}

function userContentToResponses(content: UserContentBlock[]): CodexUserContent[] {
  return content.flatMap((block): CodexUserContent[] => {
    if (block.type === 'text')
      return [{ type: 'input_text', text: block.text }]
    if (block.type === 'reference')
      return [{
        type: 'input_text',
        text: block.reference
      }]
    if (block.type === 'attachment')
      return [{ type: 'input_text', text: attachmentTag(block) }]
    if (block.type === 'document') {
      // A document is a PDF the model reads natively; the file is also on the host by path.
      return [{
        type: 'input_file',
        filename: block.source.fileName,
        file_data: `data:${block.source.mediaType};base64,${Buffer.from(block.source.data).toString('base64')}`,
      }]
    }
    if (block.source.type === 'url')
      return [{
        type: 'input_image',
        image_url: block.source.url,
        detail: 'auto'
      }]
    const base64 = Buffer.from(
      block.source.data.buffer,
      block.source.data.byteOffset,
      block.source.data.byteLength
    ).toString('base64')
    return [{
      type: 'input_image',
      image_url: `data:${block.source.mediaType};base64,${base64}`,
      detail: 'auto'
    }]
  })
}

function toolResultToResponsesOutput(
  output: ToolResultContentBlock[]
): string | CodexUserContent[] {
  const images = output.filter((block) => block.type === 'image')
  const text = output.filter((block) => block.type === 'text')
    .map((block) => block.text)
    .join('\n')
  if (images.length === 0)
    return text
  const content: CodexUserContent[] = []
  if (text)
    content.push({ type: 'input_text', text })
  for (const image of images) {
    content.push({
      type: 'input_image',
      image_url: `data:${image.source.mediaType};base64,${image.source.data}`,
      detail: 'auto'
    })
  }
  return content
}

function toolToResponsesTool(tool: ToolDefinition): CodexResponseTool {
  return {
    type: 'function',
    name: tool.name,
    description: tool.description,
    parameters: tool.inputSchema,
    strict: null,
  }
}

function thinkingToReasoning(
  thinking: InferenceRequest['thinking']
): {
  effort?: string;
  summary?: string
} | undefined {
  if (!thinking || thinking.type === 'disabled' || thinking.type === 'budget')
    return undefined
  if (thinking.type === 'adaptive')
    return {
      effort: thinking.effort,
      summary: 'auto'
    }
  if (thinking.effort === 'none')
    return { effort: 'none' }
  return {
    effort: thinking.effort,
    summary: thinking.summary && thinking.summary !== 'off'
      ? thinking.summary
      : 'auto'
  }
}

function parseReasoningSignature(
  signature: string | null
): ResponsesReasoningItem | null {
  if (!signature) {
    return null
  }
  let raw: unknown
  try {
    raw = JSON.parse(signature)
  } catch {
    // Other providers use opaque, non-JSON signatures; they are not replayed here.
    return null
  }
  if (!isRecord(raw) || raw.type !== 'reasoning') {
    return null
  }
  return parseProviderData(responsesReasoningItemSchema, raw, 'Codex reasoning signature')
}

function stringifyArguments(input: unknown): string {
  return typeof input === 'string' ? input : JSON.stringify(input ?? {})
}


function errorEventFromFailedResponse(response: ResponsesFailed): ProviderEvent {
  const error = response.error
  const message = error?.message ?? 'Codex response failed'
  const rawCode = error?.code ?? error?.type ?? null
  const providerRequestId = providerRequestIdFrom(error, message)
  const providerResponseId = response.id
  return {
    type: 'error',
    message,
    code: normalizeErrorCode(rawCode, message),
    diagnostics: {
      source: 'stream',
      ...(rawCode ? { providerCode: rawCode } : {}),
      ...(providerRequestId ? { providerRequestId } : {}),
      ...(providerResponseId ? { providerResponseId } : {}),
    },
  }
}

function providerRequestIdFrom(
  value: ResponsesFailed['error'],
  message: string
): string | null {
  const explicit = value?.request_id
    ?? value?.requestId
  if (explicit)
    return explicit
  return message.match(/request ID ([A-Za-z0-9-]+)/i)?.[1] ?? null
}

function incompleteReason(response: ResponsesFailed): string {
  return response.incomplete_details?.reason ?? 'unknown'
}
