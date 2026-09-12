import { isRecord } from '@demicodes/utils'
import type { TokenUsage } from '@demicodes/core'
import type { DisplayedBlock as Block } from '@demicodes/agent/client'
import { Allow, parse } from 'partial-json'
import { shouldParsePartialToolInput } from './tool-rendering'

type ToolCallBlock = Extract<Block, { type: 'tool_call' }>

export interface ShellTerminalOutputChunk {
  stream: 'stdout' | 'stderr'
  text: string
}

export function getLatestResponseUsage(blocks: readonly Block[]): TokenUsage | null {
  for (let i = blocks.length - 1; i >= 0; i--) {
    const block = blocks[i]!
    if (block.type === 'response')
      return block.usage
  }
  return null
}

export function getToolErrorText(block: ToolCallBlock): string | undefined {
  if (block.status !== 'error')
    return undefined
  const texts: string[] = []
  for (const part of block.output) {
    if (part.type === 'text')
      texts.push(part.text)
  }
  return texts.length > 0 ? texts.join('\n') : undefined
}

export function toolOutputText(block: ToolCallBlock): string {
  const source = block.status === 'executing'
    ? block.streamingOutput
    : block.streamingOutput.length > 0 ? block.streamingOutput : block.output
  return source
    .filter((part): part is Extract<typeof part, { type: 'text' }> => part.type === 'text')
    .map((part) => part.text)
    .join('\n')
}

export function shellTerminalOutputChunks(block: ToolCallBlock): ShellTerminalOutputChunk[] {
  return outputChunks(block.view)
}

/**
 * A tool call's input as an object while it may still be streaming: standard
 * tools parse the partial JSON so their row can name the command early; any
 * other tool waits for the complete document.
 */
export function parseToolCallInput(block: ToolCallBlock): Record<string, unknown> {
  return readToolInput(block.input, shouldParsePartialToolInput(block.toolName))
}

export function parseToolInput(raw: string): Record<string, unknown> {
  return readToolInput(raw, false)
}

function readToolInput(raw: string, partial: boolean): Record<string, unknown> {
  if (!raw) {
    return {}
  }
  try {
    const value: unknown = partial ? parse(raw, Allow.ALL) : JSON.parse(raw)
    return isRecord(value) ? value : {}
  } catch {
    // Presentation can lack input while JSON is incomplete; execution validates separately.
    return {}
  }
}

function outputChunks(view: unknown): ShellTerminalOutputChunk[] {
  if (!isRecord(view) || !Array.isArray(view['chunks']))
    return []
  return view['chunks'].flatMap((chunk): ShellTerminalOutputChunk[] => {
    if (!isRecord(chunk))
      return []
    const stream = chunk['stream']
    const text = chunk['text']
    if ((stream !== 'stdout' && stream !== 'stderr') ||
      typeof text !== 'string' ||
      text.length === 0)
      return []
    return [{ stream, text }]
  })
}
