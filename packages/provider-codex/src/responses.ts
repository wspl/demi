import { attachmentTag } from '@demicodes/core'
import { shortHash } from '@demicodes/utils'
import { Buffer } from 'node:buffer'
import type { ToolResultContentBlock, UserContentBlock } from '@demicodes/core'
import {
  clampPromptCacheKey,
  responsesReasoningItemSchema,
  type InferenceItem,
  type InferenceRequest,
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
      encrypted_content?: string
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

/**
 * The reasoning item a thinking block carries as its signature, ready to be
 * replayed. Demi wrote the signature from a decoded reasoning item, so a
 * signature that is not one belongs to another provider's transcript: the item
 * is dropped rather than sent to the vendor as an unreadable input.
 */
function parseReasoningSignature(
  signature: string | null
): CodexResponseInputItem | null {
  if (!signature)
    return null
  let value: unknown
  try {
    value = JSON.parse(signature)
  } catch {
    return null
  }
  const item = responsesReasoningItemSchema.safeParse(value)
  return item.success ? item.data : null
}

function stringifyArguments(input: unknown): string {
  return typeof input === 'string' ? input : JSON.stringify(input ?? {})
}
