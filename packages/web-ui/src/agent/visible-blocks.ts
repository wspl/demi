import type { Block } from '@demicodes/protocol'

/**
 * The blocks the transcript shows. The hidden input, `context`, reaches the
 * model but never the reader; a `wakeup` shows as its reports' rows
 * (`runtime.md` § Block types).
 *
 * A compaction shows where it was triggered: at its marker, appended at the
 * end when the pass finished. Its boundary, which the pass inserted earlier,
 * where the kept history begins, shows only when an edit removed the marker.
 */
export function getVisibleBlocks(blocks: readonly Block[]): Block[] {
  const marked = markedBoundaries(blocks)
  const visible: Block[] = []
  for (const block of blocks) {
    if (
      block.type === 'redacted_thinking'
      || block.type === 'response'
      || block.type === 'resume'
      || block.type === 'context'
    ) {
      continue
    }
    if (block.type === 'compaction_boundary' && marked.has(block.id)) {
      continue
    }
    if (block.type === 'abort' && block.isResumed) {
      continue
    }
    visible.push(block)
  }
  return visible
}

/**
 * The summary size each compaction divider tells, by the id of the block
 * that shows it: a marker tells its boundary's, and a boundary without a
 * marker its own.
 */
export function compactionSummaryTokens(blocks: readonly Block[]): Map<string, number> {
  const boundaries = new Map<string, number>()
  for (const block of blocks) {
    if (block.type === 'compaction_boundary') {
      boundaries.set(block.id, block.summaryTokens)
    }
  }
  const dividers = new Map(boundaries)
  for (const block of blocks) {
    if (block.type !== 'compaction_marker') {
      continue
    }
    const tokens = boundaries.get(block.boundaryId)
    if (tokens !== undefined) {
      dividers.set(block.id, tokens)
    }
  }
  return dividers
}

/** Whether a block is a compaction's boundary or marker, which ends no turn. */
export function isCompactionDivider(block: { type: string }): boolean {
  return block.type === 'compaction_marker' || block.type === 'compaction_boundary'
}

function markedBoundaries(blocks: readonly Block[]): Set<string> {
  const marked = new Set<string>()
  for (const block of blocks) {
    if (block.type === 'compaction_marker') {
      marked.add(block.boundaryId)
    }
  }
  return marked
}
