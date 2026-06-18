import { Buffer } from 'node:buffer'
import type { InferenceItem, InferenceRequest, ToolDefinition } from '@demi/provider'
import type { DocumentSource, ImageSource, ToolResultContentBlock, UserContentBlock } from '@demi/core'

export interface ClaudeInputMessage {
  type: 'user' | 'assistant' | 'control_response'
  message?: {
    role: 'user' | 'assistant'
    content: unknown[]
  }
  id?: string | number
  response?: unknown
}

export function requestToInputMessages(request: InferenceRequest): ClaudeInputMessage[] {
  const messages: ClaudeInputMessage[] = []
  let pending: ClaudeInputMessage | null = null
  let pendingUserKind: 'user_message' | 'tool_result' | null = null

  const flush = (): void => {
    if (pending) messages.push(pending)
    pending = null
    pendingUserKind = null
  }

  for (const item of request.items) {
    switch (item.type) {
      case 'user_message':
        flush()
        messages.push({ type: 'user', message: { role: 'user', content: userContentToClaude(item.content) } })
        break
      case 'assistant_text':
      case 'assistant_thinking':
      case 'assistant_redacted_thinking':
      case 'tool_use': {
        if (pending?.type !== 'assistant') {
          flush()
          pending = { type: 'assistant', message: { role: 'assistant', content: [] } }
        }
        pending.message?.content.push(assistantItemToClaudeContent(item))
        break
      }
      case 'tool_result':
        if (pending?.type !== 'user' || pendingUserKind !== 'tool_result') {
          flush()
          pending = { type: 'user', message: { role: 'user', content: [] } }
          pendingUserKind = 'tool_result'
        }
        pending.message?.content.push(toolResultToClaudeContent(item))
        break
    }
  }
  flush()
  return messages
}

export function inferenceItemToClaudeMessage(item: InferenceItem): ClaudeInputMessage | null {
  switch (item.type) {
    case 'user_message':
      return { type: 'user', message: { role: 'user', content: userContentToClaude(item.content) } }
    case 'assistant_text':
      return { type: 'assistant', message: { role: 'assistant', content: [assistantItemToClaudeContent(item)] } }
    case 'assistant_thinking':
      return { type: 'assistant', message: { role: 'assistant', content: [assistantItemToClaudeContent(item)] } }
    case 'assistant_redacted_thinking':
      return { type: 'assistant', message: { role: 'assistant', content: [assistantItemToClaudeContent(item)] } }
    case 'tool_use':
      return { type: 'assistant', message: { role: 'assistant', content: [assistantItemToClaudeContent(item)] } }
    case 'tool_result':
      return toolResultsToClaudeMessage([item])
  }
  return null
}

export function toolResultsToClaudeMessage(results: Array<Extract<InferenceItem, { type: 'tool_result' }>>): ClaudeInputMessage {
  return {
    type: 'user',
    message: {
      role: 'user',
      content: results.map(toolResultToClaudeContent),
    },
  }
}

export function inputMessagesToJsonl(messages: ClaudeInputMessage[]): string {
  return messages.map((message) => JSON.stringify(message)).join('\n')
}

export function toolsListResponse(tools: ToolDefinition[]): unknown {
  return {
    tools: tools.map((tool) => ({
      name: tool.name,
      description: tool.description,
      inputSchema: tool.inputSchema,
    })),
  }
}

export function controlResponse(id: string | number, response: unknown): ClaudeInputMessage {
  return { type: 'control_response', id, response }
}

function userContentToClaude(content: UserContentBlock[]): unknown[] {
  return content.map((block) => {
    if (block.type === 'text') return { type: 'text', text: block.text }
    if (block.type === 'image') return { type: 'image', source: imageSourceToClaude(block.source) }
    if (block.type === 'document') return documentSourceToClaude(block.source)
    return { type: 'text', text: block.reference }
  })
}

function assistantItemToClaudeContent(
  item: Extract<InferenceItem, { type: 'assistant_text' | 'assistant_thinking' | 'assistant_redacted_thinking' | 'tool_use' }>,
): unknown {
  if (item.type === 'assistant_text') return { type: 'text', text: item.text }
  if (item.type === 'assistant_thinking') {
    return { type: 'thinking', thinking: item.text, signature: item.signature }
  }
  if (item.type === 'assistant_redacted_thinking') return { type: 'redacted_thinking', data: item.data }
  return { type: 'tool_use', id: item.toolUseId, name: item.toolName, input: item.input }
}

function toolResultToClaudeContent(item: Extract<InferenceItem, { type: 'tool_result' }>): unknown {
  return {
    type: 'tool_result',
    tool_use_id: item.toolUseId,
    is_error: item.isError,
    content: toolResultToText(item.output),
  }
}

function imageSourceToClaude(source: ImageSource): unknown {
  if (source.type === 'url') return { type: 'url', url: source.url }
  return {
    type: 'base64',
    media_type: source.mediaType,
    data: bytesToBase64(source.data),
  }
}

function documentSourceToClaude(source: DocumentSource): unknown {
  return {
    type: 'document',
    source: {
      type: 'base64',
      media_type: source.mediaType,
      data: bytesToBase64(source.data),
    },
    title: source.fileName,
  }
}

function toolResultToText(output: ToolResultContentBlock[]): string {
  return output.map((block) => (block.type === 'text' ? block.text : `[image:${block.source.mediaType}]`)).join('\n')
}

function bytesToBase64(data: Uint8Array): string {
  return Buffer.from(data.buffer, data.byteOffset, data.byteLength).toString('base64')
}
