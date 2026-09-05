import type { Block, SessionPhase } from '@demicodes/core'

function isTailBlockOfType(
  blocks: readonly Block[],
  phase: SessionPhase,
  index: number,
  type: Block['type'],
): boolean {
  if (phase !== 'running') return false
  if (index !== blocks.length - 1) return false
  return blocks[index]?.type === type
}

export function isThinkingBlockStreaming(blocks: readonly Block[], phase: SessionPhase, index: number): boolean {
  return isTailBlockOfType(blocks, phase, index, 'thinking')
}

export function isTextBlockStreaming(blocks: readonly Block[], phase: SessionPhase, index: number): boolean {
  return isTailBlockOfType(blocks, phase, index, 'text')
}
