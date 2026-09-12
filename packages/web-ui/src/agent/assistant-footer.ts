import type { Block, SessionPhase } from '@demicodes/core'

/** Visible transcript order distinguishes intermediate updates from final replies. */
export function assistantFooterIds(
  visibleBlocks: readonly Pick<Block, 'id' | 'type'>[],
  phase: SessionPhase,
): Set<string> {
  const ids = new Set<string>()
  for (let index = 0; index < visibleBlocks.length; index++) {
    const block = visibleBlocks[index]!
    if (block.type !== 'text') {
      continue
    }
    const next = visibleBlocks[index + 1]
    if (next?.type === 'user' || (!next && phase === 'idle')) {
      ids.add(block.id)
    }
  }
  return ids
}
