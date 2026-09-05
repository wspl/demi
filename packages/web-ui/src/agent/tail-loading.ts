import type { SessionPhase } from '@demicodes/core'
import type { MessageListBlock } from './pending-steers'
import { isQueueTailBlock } from './queued-messages'

export function shouldShowTailLoading(
  phase: SessionPhase,
  transcriptBlocks: readonly MessageListBlock[],
  renderBlocks: readonly MessageListBlock[] = transcriptBlocks,
): boolean {
  if (phase !== 'running') return false
  if (hasActiveOutput(transcriptBlocks)) return false
  if (renderBlocks.length === 0) return true

  const last = lastNonQueueBlock(renderBlocks)
  if (!last) return true
  if (last.type === 'tool_call') return last.status !== 'executing'

  return (
    last.type === 'user'
    || last.type === 'steer'
    || last.type === 'pending_steer'
    || last.type === 'compaction_boundary'
  )
}

function lastNonQueueBlock(blocks: readonly MessageListBlock[]): MessageListBlock | undefined {
  for (let i = blocks.length - 1; i >= 0; i--) {
    const block = blocks[i]
    if (block && !isQueueTailBlock(block)) return block
  }
}

function hasActiveOutput(blocks: readonly MessageListBlock[]): boolean {
  const last = blocks[blocks.length - 1]
  return (
    last?.type === 'thinking'
    || last?.type === 'text'
    || (last?.type === 'tool_call' && last.status === 'executing')
  )
}
