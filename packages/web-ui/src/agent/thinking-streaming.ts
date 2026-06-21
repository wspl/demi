import type { Block, SessionPhase } from '@demi/core'

export function isThinkingBlockStreaming(blocks: readonly Block[], phase: SessionPhase, index: number): boolean {
  if (phase !== 'running') return false
  if (index !== blocks.length - 1) return false
  return blocks[index]?.type === 'thinking'
}
