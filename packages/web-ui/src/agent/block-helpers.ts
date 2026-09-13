import type { Block, TokenUsage } from '@demicodes/core'
import type { ShellFileChange, ShellToolView } from '@demicodes/agent'
import { shellToolViewSchema } from '@demicodes/agent/client'
import { Allow, parse } from 'partial-json'
import { z } from 'zod'
import { shouldParsePartialToolInput } from './tool-rendering'

type ToolCallBlock = Extract<Block, { type: 'tool_call' }>

export type ShellTerminalOutputChunk = ShellToolView['chunks'][number]

/** A tool call's input is a JSON object; anything else is nothing to render. */
const toolInputSchema = z.record(z.string(), z.unknown())

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

/**
 * The shell view a tool call stored, or null: a shape the contract does not
 * describe is not a shell view, and shows nothing.
 */
export function storedShellView(block: ToolCallBlock): ShellToolView | null {
  const view = shellToolViewSchema.safeParse(block.view)
  return view.success ? view.data : null
}

/** The output a shell call left on its view. */
export function shellTerminalOutputChunks(block: ToolCallBlock): ShellTerminalOutputChunk[] {
  return storedShellView(block)?.chunks ?? []
}

/**
 * A tool call's input as an object while it may still be streaming: standard
 * tools parse the partial JSON so their row can name the command early; any
 * other tool waits for the complete document.
 */
export function parseToolCallInput(block: ToolCallBlock): Record<string, unknown> {
  if (!block.input)
    return {}
  try {
    const result = shouldParsePartialToolInput(block.toolName)
      ? parse(block.input, Allow.ALL)
      : JSON.parse(block.input)
    const input = toolInputSchema.safeParse(result)
    return input.success ? input.data : {}
  } catch {
    return {}
  }
}

export function parseToolInput(raw: string): Record<string, unknown> {
  if (!raw)
    return {}
  try {
    const input = toolInputSchema.safeParse(JSON.parse(raw))
    return input.success ? input.data : {}
  } catch {
    return {}
  }
}

export type { ShellFileChange }
