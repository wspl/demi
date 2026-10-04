import type { Block, SessionPhase } from '@demicodes/protocol'

/**
 * Visible transcript order distinguishes intermediate updates from final
 * replies. A compaction's divider after a reply leaves it final: the user
 * compacted after it, and nothing of the turn followed.
 */
export function assistantFooterIds(
  visibleBlocks: readonly Pick<Block, 'id' | 'type'>[],
  phase: SessionPhase,
): Set<string> {
  const turnBlocks = visibleBlocks.filter((block) => !isCompactionDivider(block))
  const ids = new Set<string>()
  for (let index = 0; index < turnBlocks.length; index++) {
    const block = turnBlocks[index]!
    if (block.type !== 'text') {
      continue
    }
    const next = turnBlocks[index + 1]
    if (next?.type === 'user' || (!next && phase === 'idle')) {
      ids.add(block.id)
    }
  }
  return ids
}

function isCompactionDivider(block: Pick<Block, 'type'>): boolean {
  return block.type === 'compaction_marker' || block.type === 'compaction_boundary'
}
