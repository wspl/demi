import {
  latestWindow,
  windowEnd,
  windowOf,
  type HeldBlock,
  type HeldTranscript,
  type TranscriptWindow,
} from '@demicodes/conversation-client'

/**
 * The blocks at the transcript's end, its latest window, which the stream
 * keeps live (`web-application.md` § Transcript windows): what a running
 * turn, the message offered to edit and the tail rows read.
 */
export function latestBlocks(history: HeldTranscript): HeldBlock[] {
  return latestWindow(history)?.blocks ?? []
}

/** A block to bring into view and mark for a moment, in the transcript of the agent `node`, the root's for null. */
export interface TranscriptReveal {
  node: string | null
  blockId: string
}

/**
 * The window the transcript shows: the one that holds `shownAt`, the block
 * the reader went to, or else the latest; none held yet, an empty one at
 * the start.
 */
export function shownWindow(history: HeldTranscript, shownAt: string | null): TranscriptWindow {
  return (shownAt === null ? undefined : windowOf(history, shownAt))
    ?? latestWindow(history)
    ?? history.windows.at(-1)
    ?? { start: 0, blocks: [] }
}

/** Where a window stands in its transcript: whether it reaches the start, the end, and is the latest. */
export function windowEdges(history: HeldTranscript, window: TranscriptWindow): { atStart: boolean; atEnd: boolean } {
  return { atStart: window.start === 0, atEnd: windowEnd(window) >= history.length }
}

/** A whole transcript held at once, as a gallery specimen or a test gives it. */
export function wholeHistory(blocks: HeldBlock[]): HeldTranscript {
  return { length: blocks.length, windows: [{ start: 0, blocks }] }
}
