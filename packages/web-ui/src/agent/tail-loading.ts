import type { SessionPhase } from '@demi/core'
import type { MessageListBlock } from './pending-steers'

export function shouldShowTailLoading(phase: SessionPhase, blocks: readonly MessageListBlock[]): boolean {
  if (phase !== 'running') return false
  if (blocks.length === 0) return true

  const last = blocks[blocks.length - 1]!
  if (last.type === 'tool_call') return last.status !== 'executing'

  return (
    last.type === 'user'
    || last.type === 'pending_steer'
    || last.type === 'resume'
    || last.type === 'response'
    || last.type === 'compaction_boundary'
  )
}
