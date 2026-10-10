import type { Block, TranscriptPatch } from '@demicodes/protocol'

/**
 * A block as the page holds it: whole, as the stream and a whole read send
 * it, or `light`, as a page of the transcript sends it, without what only
 * its open row shows (`web-api.md` § Light form).
 */
export type HeldBlock = Block & { light?: true }

/** Consecutive blocks of a transcript the page holds, from the index `start`. */
export interface TranscriptWindow {
  start: number
  blocks: HeldBlock[]
}

/**
 * The parts of one agent's transcript the page holds
 * (`web-application.md` § Transcript windows): windows in order, none
 * meeting another, as windows that meet are joined; and how many blocks
 * the transcript holds, so a window knows whether it reaches the end.
 */
export interface HeldTranscript {
  length: number
  windows: readonly TranscriptWindow[]
}

export const EMPTY_TRANSCRIPT: HeldTranscript = { length: 0, windows: [] }

/** The index just past a window's last block. */
export function windowEnd(window: TranscriptWindow): number {
  return window.start + window.blocks.length
}

/** The window that reaches the transcript's end, which the stream keeps live. */
export function latestWindow(held: HeldTranscript): TranscriptWindow | undefined {
  const last = held.windows.at(-1)
  return last && windowEnd(last) >= held.length ? last : undefined
}

/** The window that holds the block `id`. */
export function windowOf(held: HeldTranscript, id: string): TranscriptWindow | undefined {
  return held.windows.find((window) => window.blocks.some((block) => block.id === id))
}

/** Every block the page holds, in order, the windows' one after another. */
export function heldBlocks(held: HeldTranscript): HeldBlock[] {
  return held.windows.flatMap((window) => window.blocks)
}

/**
 * The blocks the page names when it opens or syncs the stream
 * (`runtime.md` § Where a reset starts): the first block of its latest
 * window that may have changed since it read it, its first call that was
 * executing, or else its last block; none without a latest window.
 */
export function heldEdge(held: HeldTranscript): { from: number; edge: string } | undefined {
  const latest = latestWindow(held)
  if (!latest || latest.blocks.length === 0) {
    return undefined
  }
  const executing = latest.blocks.findIndex((block) => block.type === 'tool_call' && block.status === 'executing')
  const offset = executing >= 0 ? executing : latest.blocks.length - 1
  return { from: latest.start + offset, edge: latest.blocks[offset]!.id }
}

/**
 * `windows` with `incoming` among them, joined with each window it meets.
 * Where the two hold the same index, `prefer` says whose block stays: the
 * held one when a page arrives, since the stream's is newer; the incoming
 * one for a reset, which is the stream's. An empty incoming window joins
 * only for the stream, whose end the page holds even when nothing is there
 * yet.
 */
function joined(
  windows: readonly TranscriptWindow[],
  incoming: TranscriptWindow,
  prefer: 'held' | 'incoming',
): TranscriptWindow[] {
  if (incoming.blocks.length === 0 && prefer === 'held') {
    return [...windows]
  }
  const before: TranscriptWindow[] = []
  const after: TranscriptWindow[] = []
  let start = incoming.start
  let end = windowEnd(incoming)
  const meeting: TranscriptWindow[] = []
  for (const window of windows) {
    if (windowEnd(window) < incoming.start) {
      before.push(window)
    } else if (window.start > windowEnd(incoming)) {
      after.push(window)
    } else {
      meeting.push(window)
      start = Math.min(start, window.start)
      end = Math.max(end, windowEnd(window))
    }
  }
  const blocks: HeldBlock[] = new Array(end - start)
  const place = (window: TranscriptWindow) => {
    window.blocks.forEach((block, offset) => {
      blocks[window.start - start + offset] = block
    })
  }
  if (prefer === 'held') {
    place(incoming)
    meeting.forEach(place)
  } else {
    meeting.forEach(place)
    place(incoming)
  }
  return [...before, { start, blocks }, ...after]
}

/**
 * The transcript with the stream's reset in it (`runtime.md` § Where a
 * reset starts): its blocks replace every held block from `start` on.
 * A reset that starts elsewhere than at `asked`, the index the page named,
 * means the transcript was rewritten under the page: the page then keeps
 * only the reset (`web-application.md` § Rewrites).
 */
export function resetTranscript(
  held: HeldTranscript,
  reset: { start: number; length: number; blocks: Block[] },
  asked?: number,
): HeldTranscript {
  const rewritten = asked !== undefined && reset.start !== asked
  const kept = rewritten ? [] : truncated(held.windows, reset.start)
  return {
    length: reset.length,
    windows: joined(kept, { start: reset.start, blocks: [...reset.blocks] }, 'incoming'),
  }
}

/**
 * The transcript with a page of it the page read (`web-api.md` § Pages),
 * its blocks light. A block the page already holds stays as it is. A page
 * tells the transcript's length as the database holds it, which the page
 * takes only while no stream told it one.
 */
export function addPage(
  held: HeldTranscript,
  page: { start: number; length: number; blocks: Block[] },
  live: boolean,
): HeldTranscript {
  const blocks = page.blocks.map((block): HeldBlock => ({ ...block, light: true }))
  return {
    length: live ? held.length : page.length,
    windows: joined(held.windows, { start: page.start, blocks }, 'held'),
  }
}

/** The transcript with `block` whole in place of the held one of its id. */
export function withWholeBlock(held: HeldTranscript, block: Block): HeldTranscript {
  return {
    length: held.length,
    windows: held.windows.map((window) => {
      const offset = window.blocks.findIndex((candidate) => candidate.id === block.id)
      if (offset < 0) {
        return window
      }
      const blocks = [...window.blocks]
      blocks[offset] = block
      return { start: window.start, blocks }
    }),
  }
}

/** `windows` without the blocks from `length` on. */
function truncated(windows: readonly TranscriptWindow[], length: number): TranscriptWindow[] {
  return windows.flatMap((window) => {
    if (window.start >= length) {
      return []
    }
    if (windowEnd(window) <= length) {
      return [window]
    }
    return [{ start: window.start, blocks: window.blocks.slice(0, length - window.start) }]
  })
}

/**
 * The one transcript patch applier (`contracts.md` § Generated TypeScript):
 * `held` with `patches` applied in order. A patch changes the blocks the
 * page holds; one at a block it does not hold changes nothing it shows,
 * and an `add` or a `truncate` moves the indices after it either way
 * (`runtime.md` § Where a reset starts). A block a patch touches is
 * replaced, never changed in place, so snapshots can share blocks.
 */
export function applyTranscriptPatches(held: HeldTranscript, patches: readonly TranscriptPatch[]): HeldTranscript {
  let length = held.length
  let windows = [...held.windows]
  for (const patch of patches) {
    switch (patch.op) {
      case 'truncate': {
        // The stream keeps the end live: a page that held it holds the new
        // end, even when the cut leaves nothing there.
        const live = latestWindow({ length, windows }) !== undefined
        length = patch.length
        windows = truncated(windows, patch.length)
        if (live && latestWindow({ length, windows }) === undefined) {
          windows = joined(windows, { start: length, blocks: [] }, 'incoming')
        }
        break
      }
      case 'add':
        length += 1
        windows = windows.map((window) => {
          if (patch.index < window.start) {
            return { start: window.start + 1, blocks: window.blocks }
          }
          if (patch.index > windowEnd(window)) {
            return window
          }
          const blocks = [...window.blocks]
          blocks.splice(patch.index - window.start, 0, patch.value)
          return { start: window.start, blocks }
        })
        // A block added in a gap that the page does not hold joins no window.
        break
      case 'replace_block':
      case 'append_text':
        windows = windows.map((window) => {
          const offset = patch.index - window.start
          if (offset < 0 || offset >= window.blocks.length) {
            return window
          }
          const block = window.blocks[offset]!
          const next = patch.op === 'replace_block'
            ? patch.value
            : (block.type === 'text' || block.type === 'thinking')
              ? { ...block, text: block.text + patch.delta }
              : block
          const blocks = [...window.blocks]
          blocks[offset] = next
          return { start: window.start, blocks }
        })
        break
    }
  }
  return { length, windows }
}
