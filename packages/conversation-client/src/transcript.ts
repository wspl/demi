import type { Block, BlockVersions, HeldEdge, TranscriptPatch, Version, Versions } from '@demicodes/protocol'

/** A text or thinking block's text up to `end` reached the page at `at`, its `performance.now()`. */
export interface TextArrival {
  end: number
  at: number
}

/**
 * A block as the page holds it: whole, as the stream and a whole read send it,
 * or `light`, as a page of the transcript sends it, without what only its open
 * row shows.
 *
 * - A text or thinking block the stream brings keeps when each piece of its text
 *   arrived, `arrivals`, so a row that shows it fades each piece from then, even
 *   a row that mounts later. A block read whole has none, and a replaced block
 *   loses them.
 * - A block whose place on the path holds other versions keeps where it stands
 *   among them, `versions`.
 */
export type HeldBlock = Block & { light?: true; arrivals?: readonly TextArrival[]; versions?: Versions; stale?: true }

/** `blocks`, each with its versions that `versions` lists by block id. */
function withVersions(blocks: readonly HeldBlock[], versions: readonly BlockVersions[] | undefined): HeldBlock[] {
  if (!versions || versions.length === 0) {
    return [...blocks]
  }
  const byId = new Map(versions.map(({ blockId, index, count }) => [blockId, { index, count }]))
  return blocks.map((block) => {
    const held = byId.get(block.id)
    return held ? { ...block, versions: held } : block
  })
}

/** `block` with its text, as it is now, marked as arrived at `at`. */
function arrived(block: HeldBlock, at: number): HeldBlock {
  if ((block.type !== 'text' && block.type !== 'thinking') || block.text === '') {
    return block
  }
  return { ...block, arrivals: [...(block.arrivals ?? []), { end: block.text.length, at }] }
}

/**
 * What a page holds at an index of a transcript: a block, or, in a share's, a
 * row of its own in its place.
 *
 * A `stale` one was held before a reset that could not vouch for it, as after
 * a long time away or a rewrite under the page: it stays on screen, a hole
 * the page reads again where it shows it, and the read replaces it.
 */
export interface Held {
  id: string
  stale?: true
}

/** Consecutive blocks of a transcript the page holds, from the index `start`. */
export interface TranscriptWindow<B extends Held = HeldBlock> {
  start: number
  blocks: B[]
}

/**
 * The parts of one agent's transcript the page holds: windows in order, none
 * meeting another, as windows that meet are joined; and how many blocks the
 * transcript holds, so a window knows whether it reaches the end.
 */
export interface HeldTranscript<B extends Held = HeldBlock> {
  length: number
  windows: readonly TranscriptWindow<B>[]
  /**
   * The version of the transcript its latest window is current at, as the
   * stream's last reset or patch left it; absent while no stream vouched for
   * it, as for a window only pages were read into.
   */
  version?: Version
}

/**
 * A transcript as the page holds it, frozen: it is replaced at each change,
 * never changed in place, so whoever holds it follows the one reference.
 *
 * A frozen object is also one a reactive store keeps as it is rather than behind
 * a proxy (Vue makes no proxy of an object that cannot be extended). Reading a
 * long transcript at each streamed piece then costs plain reads, not a tracked
 * read of every field.
 */
export function heldTranscript<B extends Held>(
  length: number,
  windows: readonly TranscriptWindow<B>[],
  version?: Version,
): HeldTranscript<B> {
  return Object.freeze(version ? { length, windows, version } : { length, windows })
}

export const EMPTY_TRANSCRIPT: HeldTranscript = heldTranscript(0, [])

/** The index just past a window's last block. */
export function windowEnd<B extends Held>(window: TranscriptWindow<B>): number {
  return window.start + window.blocks.length
}

/** The window that reaches the transcript's end, which the stream keeps live. */
export function latestWindow<B extends Held>(held: HeldTranscript<B>): TranscriptWindow<B> | undefined {
  const last = held.windows.at(-1)
  return last && windowEnd(last) >= held.length ? last : undefined
}

/** The window that holds the block `id`. */
export function windowOf<B extends Held>(held: HeldTranscript<B>, id: string): TranscriptWindow<B> | undefined {
  return held.windows.find((window) => window.blocks.some((block) => block.id === id))
}

/** Every block the page holds, in order, the windows' one after another. */
export function heldBlocks<B extends Held>(held: HeldTranscript<B>): B[] {
  return held.windows.flatMap((window) => window.blocks)
}

/**
 * The blocks the page names when it subscribes to a node: the version its
 * latest window is current at, and the first block of that window that may
 * have changed since, its first call that was executing, or else its last
 * block; none without a latest window a stream vouched for.
 */
export function heldEdge(held: HeldTranscript): HeldEdge | undefined {
  const latest = latestWindow(held)
  if (!held.version || !latest || latest.blocks.length === 0) {
    return undefined
  }
  const executing = latest.blocks.findIndex((block) => block.type === 'tool_call' && block.status === 'executing')
  const offset = executing >= 0 ? executing : latest.blocks.length - 1
  return { version: { ...held.version }, from: latest.start + offset, edge: latest.blocks[offset]!.id }
}

/**
 * `windows` with `incoming` among them, joined with each window it meets.
 *
 * Where the two hold the same index, `prefer` says whose block stays:
 *
 * - The held one when a page arrives, since the stream's is newer.
 * - The incoming one for a reset, which is the stream's.
 *
 * An empty incoming window joins only for the stream, whose end the page holds
 * even when nothing is there yet.
 */
function joined<B extends Held>(
  windows: readonly TranscriptWindow<B>[],
  incoming: TranscriptWindow<B>,
  prefer: 'held' | 'incoming',
): TranscriptWindow<B>[] {
  if (incoming.blocks.length === 0 && prefer === 'held') {
    return [...windows]
  }
  const before: TranscriptWindow<B>[] = []
  const after: TranscriptWindow<B>[] = []
  let start = incoming.start
  let end = windowEnd(incoming)
  const meeting: TranscriptWindow<B>[] = []
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
  const blocks: B[] = new Array(end - start)
  const place = (window: TranscriptWindow<B>) => {
    window.blocks.forEach((block, offset) => {
      blocks[window.start - start + offset] = block
    })
  }
  if (prefer === 'held') {
    place(incoming)
    // A stale block is a hole: what arrives replaces it.
    for (const window of meeting) {
      window.blocks.forEach((block, offset) => {
        const at = window.start - start + offset
        if (!block.stale || blocks[at] === undefined) {
          blocks[at] = block
        }
      })
    }
  } else {
    meeting.forEach(place)
    place(incoming)
  }
  return [...before, { start, blocks }, ...after]
}

/**
 * The transcript with the stream's reset in it: its blocks replace every
 * held block from `start` on, light as a page's are but the last, which the
 * stream may still be writing and sends whole.
 *
 * What the page holds before `start` stays on screen. The reset vouches for
 * it when it starts at `asked`, the index the page named, whose block the
 * server found there unchanged; otherwise, as after a long time away, a
 * rewrite under the page, or a page that named no index, it is stale: a hole
 * the page reads again where it shows it.
 */
export function resetTranscript(
  held: HeldTranscript,
  reset: { start: number; length: number; blocks: Block[]; versions?: readonly BlockVersions[]; version?: Version },
  asked?: number,
): HeldTranscript {
  const vouched = asked !== undefined && reset.start === asked
  const kept = truncated(held.windows, reset.start).map((window) => vouched
    ? window
    : { start: window.start, blocks: window.blocks.map((block): HeldBlock => (block.stale ? block : { ...block, stale: true })) })
  const last = reset.blocks.length - 1
  const blocks = reset.blocks.map((block, offset): HeldBlock => (offset === last ? block : { ...block, light: true }))
  return heldTranscript(
    reset.length,
    joined(kept, { start: reset.start, blocks: withVersions(blocks, reset.versions) }, 'incoming'),
    reset.version,
  )
}

/**
 * The transcript with a page of it the page read, its blocks light. A block the page
 * already holds stays as it is. A page tells the transcript's length as the database
 * holds it, which the page takes only while no stream told it one.
 */
export function addPage(
  held: HeldTranscript,
  page: { start: number; length: number; blocks: Block[]; versions?: readonly BlockVersions[] },
  live: boolean,
): HeldTranscript {
  const blocks = withVersions(page.blocks.map((block): HeldBlock => ({ ...block, light: true })), page.versions)
  return addWindow(held, { start: page.start, length: page.length, blocks }, live)
}

/**
 * The transcript with `page`'s items, as the page holds them, among its
 * windows: an item the page already holds stays as it is, and the page's
 * length is taken only while no stream told one.
 */
export function addWindow<B extends Held>(
  held: HeldTranscript<B>,
  page: { start: number; length: number; blocks: B[] },
  live: boolean,
): HeldTranscript<B> {
  return heldTranscript(live ? held.length : page.length, joined(held.windows, { start: page.start, blocks: page.blocks }, 'held'), held.version)
}

/**
 * The transcript with only the blocks a view needs: at most `half` blocks on
 * each side of the block `anchor` in the window that holds it, and the last
 * `half` blocks of the latest window, which the stream keeps live and the
 * conversation's status reads; for an `anchor` null or not held, the latest
 * window's alone, else the last window's. Every other window and block
 * leaves memory, as Telegram's history views drop what falls outside them and
 * its chat list keeps each chat's top message
 * (`docs/decisions/0025-a-transcript-is-shown-from-a-bounded-window.md`).
 * One already within the bound is returned as it is.
 *
 * `also` names indices that stay too, the page a read just brought: the
 * anchor the view last told may lie a screen away from it, and the page the
 * reader is about to reach is not cut as it arrives.
 */
export function boundedTranscript<B extends Held>(
  held: HeldTranscript<B>,
  anchor: string | null,
  half: number,
  also?: { start: number; end: number },
): HeldTranscript<B> {
  const tail = latestWindow(held) ?? held.windows.at(-1)
  if (!tail) {
    return held
  }
  /** The ranges kept of each window, as offsets from its start. */
  const kept = new Map<TranscriptWindow<B>, [number, number][]>()
  const keep = (window: TranscriptWindow<B>, from: number, to: number) => {
    kept.set(window, [...kept.get(window) ?? [], [from, to]])
  }
  keep(tail, Math.max(0, tail.blocks.length - half), tail.blocks.length)
  const anchored = anchor === null ? undefined : windowOf(held, anchor)
  if (anchored) {
    const at = anchored.blocks.findIndex((block) => block.id === anchor)
    keep(anchored, Math.max(0, at - half), Math.min(anchored.blocks.length, at + half + 1))
  }
  if (also) {
    for (const window of held.windows) {
      const from = Math.max(also.start - window.start, 0)
      const to = Math.min(also.end - window.start, window.blocks.length)
      if (from < to) {
        keep(window, from, to)
      }
    }
  }
  const windows = held.windows.flatMap((window) => {
    const ranges = (kept.get(window) ?? []).sort((a, b) => a[0] - b[0])
    // Ranges of one window that meet are one run; apart, a gap lies between them.
    const runs = ranges.reduce<[number, number][]>((joined, range) => {
      const last = joined.at(-1)
      if (last && range[0] <= last[1]) {
        last[1] = Math.max(last[1], range[1])
      } else {
        joined.push([...range])
      }
      return joined
    }, [])
    return runs.map(([from, to]) => ({ start: window.start + from, blocks: window.blocks.slice(from, to) }))
  })
  if (windows.length === held.windows.length && windows.every((window, index) => window.blocks.length === held.windows[index]!.blocks.length)) {
    return held
  }
  return heldTranscript(held.length, windows, held.version)
}

/** The transcript with `block` whole in place of the held one of its id. */
export function withWholeBlock(held: HeldTranscript, block: Block): HeldTranscript {
  return heldTranscript(held.length, held.windows.map((window) => {
    const offset = window.blocks.findIndex((candidate) => candidate.id === block.id)
    if (offset < 0) {
      return window
    }
    const blocks = [...window.blocks]
    blocks[offset] = block
    return { start: window.start, blocks }
  }), held.version)
}

/** `windows` without the blocks from `length` on. */
function truncated<B extends Held>(windows: readonly TranscriptWindow<B>[], length: number): TranscriptWindow<B>[] {
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
 * The one transcript patch applier (`crates/xtask/src/contracts/mod.rs`): `held`
 * with `patches` applied in order.
 *
 * - A patch changes the blocks the page holds. One at a block it does not hold
 *   changes nothing it shows, and an `add` or a `truncate` moves the indices
 *   after it either way.
 * - A block a patch touches is replaced, never changed in place, so snapshots
 *   can share blocks.
 * - Text an `add` or an `append_text` brings is marked as arrived at
 *   `receivedAt`.
 * - `version` is the transcript's version after them, as the stream that
 *   brought them counts it; the held one stays without.
 */
export function applyTranscriptPatches(
  held: HeldTranscript,
  patches: readonly TranscriptPatch[],
  receivedAt: number = performance.now(),
  version: Version | undefined = held.version,
): HeldTranscript {
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
      case 'add': {
        const added = arrived(patch.versions ? { ...patch.value, versions: patch.versions } : patch.value, receivedAt)
        // A block added at the end that no window reaches starts a run of
        // its own; one added in a gap the page does not hold joins none.
        const own = patch.index === length && !windows.some((window) => window.start <= patch.index && patch.index <= windowEnd(window))
        length += 1
        windows = windows.map((window) => {
          if (patch.index < window.start) {
            return { start: window.start + 1, blocks: window.blocks }
          }
          if (patch.index > windowEnd(window)) {
            return window
          }
          const blocks = [...window.blocks]
          blocks.splice(patch.index - window.start, 0, added)
          return { start: window.start, blocks }
        })
        if (own) {
          windows.push({ start: patch.index, blocks: [added] })
        }
        break
      }
      case 'replace_block':
      case 'append_text':
        windows = windows.map((window) => {
          const offset = patch.index - window.start
          if (offset < 0 || offset >= window.blocks.length) {
            return window
          }
          const block = window.blocks[offset]!
          // A block replaced in place keeps its place, so its versions.
          const next = patch.op === 'replace_block'
            ? (block.versions ? { ...patch.value, versions: block.versions } : patch.value)
            : (block.type === 'text' || block.type === 'thinking')
              ? arrived({ ...block, text: block.text + patch.delta }, receivedAt)
              : block
          const blocks = [...window.blocks]
          blocks[offset] = next
          return { start: window.start, blocks }
        })
        break
    }
  }
  return heldTranscript(length, windows, version)
}
