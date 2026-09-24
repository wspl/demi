import type { Block } from '@demicodes/protocol'

/**
 * The blocks the transcript shows. The hidden inputs, `context` and `wakeup`,
 * reach the model but never the reader (`runtime.md` § Block types).
 */
export function getVisibleBlocks(blocks: readonly Block[]): Block[] {
  const visible: Block[] = []
  for (const block of blocks) {
    if (
      block.type === 'redacted_thinking'
      || block.type === 'compaction_marker'
      || block.type === 'response'
      || block.type === 'resume'
      || block.type === 'context'
      || block.type === 'wakeup'
    ) {
      continue
    }
    if (block.type === 'abort' && block.isResumed) {
      continue
    }
    visible.push(block)
  }
  return visible
}
