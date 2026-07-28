import type { Block } from '@demicodes/core'

/**
 * Where re-inference must restart after a turn failed to finish.
 *
 * `cut` is the index from which blocks are discardable; `isFullRerun` is true when
 * that reaches all the way back to the user turn, meaning the turn produced nothing
 * and can simply be run again.
 */
export interface ResumePoint {
  cut: number
  isFullRerun: boolean
}

/**
 * Whether a block is a leftover of the failed attempt that nobody can have acted on.
 *
 * Everything else has to be assumed acted on. Transcript blocks stream outward as
 * they are produced and products turn them into effects that cannot be recalled —
 * rendering them, posting them to a chat, executing the tool they describe. A tool
 * call counts whatever its status: one still marked executing outlived the process
 * that was running it, so whether its effect landed is unknown, and unknown has to
 * be treated as landed. An abort block is history the user created, not a leftover.
 *
 * Thinking is the interesting case: products display it, but nothing keys off it and
 * a rerun simply reasons again, so it is discardable. A `response` is not — it
 * records a provider request that did complete, and its usage anchors the context
 * estimate for everything after it.
 */
function isDiscardableLeftover(block: Block): boolean {
  switch (block.type) {
    case 'thinking':
    case 'redacted_thinking':
    case 'error':
      return true
    case 'text':
      return block.text.trim().length === 0
    default:
      return false
  }
}

/**
 * Finds how far back an unfinished turn can be unwound before re-inferring.
 *
 * This is the single decision behind both recovery paths. A transient provider
 * failure and a human asking to continue a dead round ask the same question — how
 * do we finish this turn — and the answer depends only on what has already left
 * the process, never on which of the two asked or on how the turn died.
 */
export function findResumePoint(blocks: readonly Block[]): ResumePoint {
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    const block = blocks[i]
    if (block.type === 'user') return { cut: i + 1, isFullRerun: true }
    if (!isDiscardableLeftover(block)) return { cut: i + 1, isFullRerun: false }
  }
  // No user turn to unwind to; keep everything and continue after it.
  return { cut: blocks.length, isFullRerun: false }
}
