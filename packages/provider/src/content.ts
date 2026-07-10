import type { ToolResultContentBlock } from '@demicodes/core'

/**
 * Flattens a tool result to plain text for wire formats without native media
 * blocks; non-text blocks become `[<type>:<mediaType>]` placeholders.
 */
export function toolResultContentToText(output: ToolResultContentBlock[]): string {
  return output.map((block) => (block.type === 'text' ? block.text : `[${block.type}:${block.source.mediaType}]`)).join('\n')
}
