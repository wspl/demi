import type { Block } from '@demi/core'

type ToolCallBlock = Extract<Block, { type: 'tool_call' }>

export function getToolErrorText(block: ToolCallBlock): string | undefined {
  if (block.status !== 'error') return undefined
  const texts: string[] = []
  for (const part of block.output) {
    if (part.type === 'text') texts.push(part.text)
  }
  return texts.length > 0 ? texts.join('\n') : undefined
}

export function toolOutputText(block: ToolCallBlock): string {
  const source = block.status === 'executing' ? block.streamingOutput : block.streamingOutput.length > 0 ? block.streamingOutput : block.output
  return source
    .filter((part): part is Extract<typeof part, { type: 'text' }> => part.type === 'text')
    .map((part) => part.text)
    .join('\n')
}

export function parseToolInput(raw: string): Record<string, unknown> {
  if (!raw) return {}
  try {
    const parsed = JSON.parse(raw)
    return typeof parsed === 'object' && parsed !== null ? (parsed as Record<string, unknown>) : {}
  } catch {
    return {}
  }
}
