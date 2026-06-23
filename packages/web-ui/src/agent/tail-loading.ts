import type { SessionPhase } from '@demi/core'
import type { MessageListBlock } from './pending-steers'

export function shouldShowTailLoading(
  phase: SessionPhase,
  transcriptBlocks: readonly MessageListBlock[],
  renderBlocks: readonly MessageListBlock[] = transcriptBlocks,
): boolean {
  if (phase !== 'running') return false
  if (hasActiveOutput(transcriptBlocks)) return false
  if (renderBlocks.length === 0) return true

  const last = renderBlocks[renderBlocks.length - 1]!
  if (last.type === 'tool_call') return last.status !== 'executing'

  return (
    last.type === 'user'
    || last.type === 'steer'
    || last.type === 'pending_steer'
    || last.type === 'resume'
    || last.type === 'response'
    || last.type === 'compaction_boundary'
  )
}

function hasActiveOutput(blocks: readonly MessageListBlock[]): boolean {
  const last = blocks[blocks.length - 1]
  return (
    last?.type === 'thinking'
    || last?.type === 'text'
    || (last?.type === 'tool_call' && last.status === 'executing')
  )
}
