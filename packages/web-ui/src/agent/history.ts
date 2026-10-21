import {
  boundedTranscript,
  latestWindow,
  windowEnd,
  windowOf,
  type Held,
  type HeldBlock,
  type HeldTranscript,
  type TranscriptWindow,
} from '@demicodes/conversation-client'

export type { HeldBlock } from '@demicodes/conversation-client'

/**
 * The blocks at the transcript's end, its latest window, which the stream
 * keeps live: what a running turn, the message offered to edit and the tail
 * rows read.
 */
export function latestBlocks(history: HeldTranscript): HeldBlock[] {
  return latestWindow(history)?.blocks ?? []
}

/**
 * A block to bring into view and mark for a moment, in the transcript of the
 * agent `node`, the root's for null.
 */
export interface TranscriptReveal {
  node: string | null
  blockId: string
}

/**
 * How many blocks a transcript shown holds in memory on each side of the block
 * its view shows; what lies further leaves memory and is read again, from the
 * disk copy first, when the view reaches it
 * (`docs/decisions/0025-a-transcript-is-shown-from-a-bounded-window.md`).
 */
export const SHOWN_HALF = 300

/**
 * `history` bounded to what a view that shows the block `shownAt`, or the
 * latest for null, holds (`SHOWN_HALF`).
 */
export function boundedHistory<B extends Held>(history: HeldTranscript<B>, shownAt: string | null): HeldTranscript<B> {
  return boundedTranscript(history, shownAt, SHOWN_HALF)
}

/**
 * The window the transcript shows: the one that holds `shownAt`, the block
 * the reader went to, or else the latest; none held yet, an empty one at
 * the start.
 */
export function shownWindow<B extends Held>(history: HeldTranscript<B>, shownAt: string | null): TranscriptWindow<B> {
  return (shownAt === null ? undefined : windowOf(history, shownAt))
    ?? latestWindow(history)
    ?? history.windows.at(-1)
    ?? { start: 0, blocks: [] }
}

/**
 * Where a window stands in its transcript: whether it reaches the start, the
 * end, and is the latest.
 */
export function windowEdges<B extends Held>(
  history: HeldTranscript<B>,
  window: TranscriptWindow<B>,
): { atStart: boolean; atEnd: boolean } {
  return { atStart: window.start === 0, atEnd: windowEnd(window) >= history.length }
}

/**
 * Where the page next to `window` on `side` is read from: the index of the
 * window's item at that edge and its id, which the page must still hold there;
 * none where the window reaches the transcript's start or end. /
 */
export function pageBeside<B extends Held>(
  history: HeldTranscript<B>,
  window: TranscriptWindow<B>,
  side: 'before' | 'after',
): { index: number; edge: string } | null {
  const edge = side === 'before' ? window.blocks[0] : window.blocks.at(-1)
  const { atStart, atEnd } = windowEdges(history, window)
  if (!edge || (side === 'before' ? atStart : atEnd)) {
    return null
  }
  return { index: side === 'before' ? window.start : windowEnd(window) - 1, edge: edge.id }
}

/**
 * A whole transcript held at once, as a gallery specimen or a test gives it.
 */
export function wholeHistory(blocks: HeldBlock[]): HeldTranscript {
  return { length: blocks.length, windows: [{ start: 0, blocks }] }
}
