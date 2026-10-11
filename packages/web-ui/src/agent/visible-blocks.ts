import type { Block } from '@demicodes/protocol'
import { reportShown } from './command-reports'
import type { MessageListBlock } from './pending-steers'

/**
 * The blocks the transcript shows. The hidden input, `context`, reaches the
 * model but never the reader; a `wakeup` shows as the rows of the ends it
 * reports, and not at all for progress alone (`runtime.md` § Block types).
 *
 * A compaction shows where it was triggered: at its marker, appended at the
 * end when the pass finished. Its boundary, which the pass inserted earlier,
 * where the kept history begins, shows only when an edit removed the marker.
 *
 * With `cuts`, the places a turn went on after it was cut are kept too
 * (`isRunCut`): they show nothing, but a run of steps ends at them.
 */
export function getVisibleBlocks(blocks: readonly Block[], { cuts = false } = {}): Block[] {
  const marked = markedBoundaries(blocks)
  const visible: Block[] = []
  for (const block of blocks) {
    if (isRunCut(block)) {
      if (cuts) {
        visible.push(block)
      }
      continue
    }
    if (
      block.type === 'redacted_thinking'
      || block.type === 'response'
      || block.type === 'context'
    ) {
      continue
    }
    if (block.type === 'compaction_boundary' && marked.has(block.id)) {
      continue
    }
    // Reports of progress show nothing; a block of them alone has no row.
    if (block.type === 'wakeup' && !block.reports.some(reportShown)) {
      continue
    }
    visible.push(block)
  }
  return visible
}

/**
 * A place where a turn went on after it was cut: a `resume` block, and a
 * Stop the user continued. It shows nothing, yet the run of steps before it
 * is done: the steps after it are a run of their own, which takes the place
 * the Stopped row or the error record had, never steps that join the run
 * above while the row under it goes (`product.md` § Following the agent's
 * work).
 */
export function isRunCut(block: MessageListBlock): boolean {
  return block.type === 'resume' || (block.type === 'abort' && block.isResumed)
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
