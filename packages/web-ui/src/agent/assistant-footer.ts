import type { Block, SessionPhase } from '@demicodes/protocol'
import { isCompactionDivider } from './visible-blocks'

/**
 * Visible transcript order distinguishes intermediate updates from final
 * replies. A compaction's divider after a reply leaves it final: the user
 * compacted after it, and nothing of the turn followed. So does the user's
 * Stop: the reply it cut off is the turn's last word, and keeps its Copy.
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
    if (next?.type === 'user' || next?.type === 'abort' || (!next && phase === 'idle')) {
      ids.add(block.id)
    }
  }
  return ids
}
