import { Allow, parse } from 'partial-json'
import { z } from 'zod'
import type { ShellToolView, ToolCallBlock } from './block-types'
import { shouldParsePartialToolInput } from './tool-rendering'

export type ShellTerminalOutputChunk = ShellToolView['chunks'][number]

/** A tool call's input is a JSON object; anything else is nothing to render. */
const toolInputSchema = z.record(z.string(), z.unknown())

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

/** The shell view a tool call stored, or null for a call of another kind. */
export function storedShellView(block: ToolCallBlock): ShellToolView | null {
  return block.view?.kind === 'shell' ? block.view : null
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
