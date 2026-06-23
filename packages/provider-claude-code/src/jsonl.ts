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
      case 'user_steer':
        flush()
        messages.push({ type: 'user', message: { role: 'user', content: userContentToClaude(item.content) } })
        break
      case 'assistant_text':
      case 'assistant_thinking':
      case 'assistant_redacted_thinking':
      case 'tool_use': {
        const content = assistantItemToClaudeContent(item)
        if (content === null) break
        if (pending?.type !== 'assistant') {
          flush()
          pending = { type: 'assistant', message: { role: 'assistant', content: [] } }
        }
        pending.message?.content.push(content)
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

/**
 * Builds the input messages used to prime a *fresh* Claude CLI process with prior
 * conversation history — i.e. on a cold start: the first turn, a resume after the process
 * died, or a restart forced by compaction.
 *
 * Two invariants:
 *
 * 1. Prior tool calls/results are rendered as plain TEXT, never structured `tool_use` /
 *    `tool_result` blocks. Replaying a structured MCP `tool_use` into a freshly-initialized
 *    SDK-MCP session is exactly what makes Claude reject the request with
 *    `API Error: 400 ... tool use concurrency`.
 * 2. A `user` message contains ONLY real user input. Both the tool call and its result are
 *    folded into the *assistant* narrative ("I ran X, it returned Y"), so we never synthesize
 *    a user turn the human did not type, and never merge a tool result into the next real
 *    prompt. This keeps the conversation cleanly alternating without fabricated user messages.
 *
 * The live, in-process path never calls this: there the CLI keeps the structured tool history
 * in its own native context, so no replay happens at all.
 */
export function coldStartInputMessages(items: InferenceItem[]): ClaudeInputMessage[] {
  const messages: ClaudeInputMessage[] = []
  let pending: { role: 'user' | 'assistant'; content: unknown[] } | null = null
  const toolNames = new Map<string, string>()

  const flush = (): void => {
    if (pending && pending.content.length > 0) {
      messages.push({ type: pending.role, message: { role: pending.role, content: pending.content } })
    }
    pending = null
  }
  const append = (role: 'user' | 'assistant', blocks: unknown[]): void => {
    if (blocks.length === 0) return
    if (!pending || pending.role !== role) {
      flush()
      pending = { role, content: [] }
    }
    pending.content.push(...blocks)
  }

  for (const item of items) {
    switch (item.type) {
      case 'user_message':
      case 'user_steer':
        append('user', userContentToClaude(item.content))
        break
      case 'assistant_text':
        append('assistant', item.text ? [{ type: 'text', text: item.text }] : [])
        break
      case 'tool_use':
        toolNames.set(item.toolUseId, item.toolName)
        append('assistant', [{ type: 'text', text: renderToolUseText(item.toolName, item.input) }])
        break
      case 'tool_result':
        // Folded into the assistant narrative, never a user turn — see invariant 2 above.
        append('assistant', [{ type: 'text', text: renderToolResultText(toolNames.get(item.toolUseId), item) }])
        break
      // assistant_thinking / assistant_redacted_thinking are intentionally skipped: thinking is
      // internal, and its signatures will not validate against a different (fresh) process.
    }
  }
  flush()
  return messages
}

function renderToolUseText(toolName: string, input: unknown): string {
  return `[Earlier in this conversation I called the tool ${toolName} with input: ${safeJson(input)}.`
}

function renderToolResultText(toolName: string | undefined, item: Extract<InferenceItem, { type: 'tool_result' }>): string {
  const body = toolResultToText(item.output)
  const suffix = toolName ? ` from ${toolName}` : ''
  return item.isError ? `It returned an error${suffix}: ${body}]` : `It returned${suffix}: ${body}]`
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value) ?? String(value)
  } catch {
    return String(value)
  }
}

export function inferenceItemToClaudeMessage(item: InferenceItem): ClaudeInputMessage | null {
  switch (item.type) {
    case 'user_message':
    case 'user_steer':
      return { type: 'user', message: { role: 'user', content: userContentToClaude(item.content) } }
    case 'assistant_text':
    case 'assistant_thinking':
    case 'assistant_redacted_thinking':
    case 'tool_use': {
      const content = assistantItemToClaudeContent(item)
      if (content === null) return null
      return { type: 'assistant', message: { role: 'assistant', content: [content] } }
    }
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
): unknown | null {
  if (item.type === 'assistant_text') return { type: 'text', text: item.text }
  if (item.type === 'assistant_thinking') {
    if (!item.signature) return null
    return { type: 'thinking', thinking: item.text, signature: item.signature }
  }
  if (item.type === 'assistant_redacted_thinking') return { type: 'redacted_thinking', data: item.data }
  return { type: 'tool_use', id: item.toolUseId, name: toolNameToClaude(item.toolName), input: item.input }
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

function toolNameToClaude(name: string): string {
  if (/^mcp__[^_]+__.+$/.test(name)) return name
  return `mcp__main__${name}`
}
