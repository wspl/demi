import type { Block } from '@demi/core'

export function getVisibleBlocks(blocks: readonly Block[]): Block[] {
  return blocks.filter((block) => {
    if (block.type === 'redacted_thinking' || block.type === 'compaction_marker' || block.type === 'extension_state_snapshot') {
      return false
    }
    if (block.type === 'abort' && block.isResumed) {
      return false
    }
    return true
  })
}
