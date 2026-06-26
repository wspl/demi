import type { Block } from '@demi/core'

export function getVisibleBlocks(blocks: readonly Block[]): Block[] {
  const visible: Block[] = []
  for (const block of blocks) {
    if (
      block.type === 'redacted_thinking'
      || block.type === 'compaction_marker'
      || block.type === 'extension_state_snapshot'
      || block.type === 'response'
      || block.type === 'resume'
    ) {
      continue
    }
    if (block.type === 'abort' && block.isResumed) {
      continue
    }
    // Hidden user/steer turns (internal yield wakeups) are replayed to the model but never rendered.
    if ((block.type === 'user' || block.type === 'steer') && block.hidden) {
      continue
    }
    visible.push(block)
  }
  return visible
}
