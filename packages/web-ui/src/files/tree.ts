/** Layout of a tree's rows: the caption and every row are this tall, with a 1px gap. */
export const TREE_ROW_PX = 28
export const TREE_ROW_PITCH_PX = TREE_ROW_PX + 1
/** A row's height on a phone (`ui/phone.ts`), a finger's target. */
export const TREE_ROW_PHONE_PX = 44

/**
 * One row of a tree as laid out: a file or directory at a depth, under a
 * parent. Paths are keys, absolute or relative alike; a directory's rows
 * follow it while it is open.
 */
export interface TreeRow {
  path: string
  name: string
  isDirectory: boolean
  depth: number
  /** The directory row above it, or null under the root. */
  parent: string | null
  /** Whether a directory shows its rows; false for a file. */
  open: boolean
}

/**
 * What a drag over a tree would drop into, lit while it is there: one of its
 * directory rows with the rows it holds, or the tree itself.
 */
export type TreeDropTarget = { kind: 'row'; path: string } | { kind: 'tree' }

/**
 * Where each row sits in the tree, by index into the rows, built once per rows so
 * that nothing a scroll or a key does walks every row: a row's directory, the
 * rows it holds, and its place among its siblings.
 *
 * The rows hold every directory's rows right after it, so the rows a directory
 * holds are the block from its row to `end`.
 */
export interface TreeIndex {
  /** Each row's index, by path. */
  indexOf: ReadonlyMap<string, number>
  /** The index of each row's directory row, or -1 for a row under the root. */
  parent: Int32Array
  /** The index of the last row each row holds, or its own when it holds none. */
  end: Int32Array
  /** Each row's place among the rows of its directory, from 1. */
  position: Int32Array
  /** How many rows its directory holds directly. */
  siblings: Int32Array
}

export function indexTree(rows: readonly TreeRow[]): TreeIndex {
  const count = rows.length
  const indexOf = new Map<string, number>()
  const parent = new Int32Array(count)
  const end = new Int32Array(count)
  const position = new Int32Array(count)
  const siblings = new Int32Array(count)
  // How many rows each directory holds directly, by its index; the root is -1.
  const held = new Map<number, number>()
  // The rows still open above the current one, deepest last.
  const open: number[] = []
  rows.forEach((row, index) => {
    indexOf.set(row.path, index)
    while (open.length > 0 && rows[open.at(-1)!]!.depth >= row.depth)
      end[open.pop()!] = index - 1
    open.push(index)
    const up = row.parent === null ? -1 : (indexOf.get(row.parent) ?? -1)
    parent[index] = up
    const place = (held.get(up) ?? 0) + 1
    held.set(up, place)
    position[index] = place
  })
  for (const index of open)
    end[index] = count - 1
  for (let index = 0; index < count; index++)
    siblings[index] = held.get(parent[index]!)!
  return { indexOf, parent, end, position, siblings }
}

/**
 * Where a tree's viewport scrolls to bring a row into view, or null when it is in
 * view already.
 *
 * - Once a row reaches the top, its ancestors pin above it, one slot per level
 *   under the caption (`captionPx`).
 * - So a row above the view, or under them, comes to rest in the slot just below
 *   them: the slot its own pinned copy takes.
 * - A row below the view comes up to its bottom edge.
 */
export function revealTreeRow(
  view: { scrollTop: number; clientHeight: number },
  row: { depth: number; top: number },
  captionPx: number,
  rowPx = TREE_ROW_PX,
): number | null {
  const belowAncestors = row.top - captionPx - row.depth * (rowPx + 1)
  const atBottom = row.top + rowPx - view.clientHeight
  if (view.scrollTop > belowAncestors)
    return belowAncestors
  if (view.scrollTop < atBottom)
    return atBottom
  return null
}

/** What to pin: the directory rows, and how far up the stack rides as the deepest one leaves. */
export interface StickyTreeStack {
  paths: string[]
  /** Zero or negative px: the stack slides up by this much while its deepest directory scrolls out. */
  offset: number
}

/**
 * The directory rows to pin at the top for a viewport scrolled to `scrollTop`,
 * where row `i` starts at `firstTop + i` pitches.
 *
 * Level by level, the row under the stack's next slot names the chain of
 * directories it sits in (itself included when it is an open directory with
 * rows under it).
 *
 * - The chain's directory for that level pins while its own row has scrolled
 *   under the slot and its last row has not, so a directory's name stays in
 *   sight while the tree scrolls through it and goes once it is past.
 * - A row outside the pinned chain ends the stack.
 * - As the deepest pinned directory's last row scrolls under the stack, the
 *   stack rides up with it instead of vanishing.
 *
 * Every step is arithmetic on the index, so the stack costs as much in a
 * directory of a hundred thousand rows as in one of ten.
 */
export function stickyTreeRows(
  rows: readonly TreeRow[],
  index: TreeIndex,
  scrollTop: number,
  firstTop: number,
  rowPx = TREE_ROW_PX,
): StickyTreeStack {
  const pitch = rowPx + 1
  const topOf = (at: number): number => firstTop + at * pitch
  const chainOf = (at: number): number[] => {
    const chain: number[] = []
    for (let up = at; up >= 0; up = index.parent[up]!)
      chain.unshift(up)
    return chain
  }
  /** The row under `y`: the first one whose bottom is below it. */
  const rowAt = (y: number): number | null => {
    const at = Math.max(0, Math.floor((y - firstTop - rowPx) / pitch) + 1)
    return at < rows.length ? at : null
  }
  const paths: string[] = []
  const pinned: number[] = []
  let offset = 0
  for (let level = 0; ; level++) {
    const slotTop = scrollTop + firstTop + level * pitch
    const anchor = rowAt(slotTop)
    if (anchor === null) {
      break
    }
    const chain = chainOf(anchor)
    // The row under the slot belongs elsewhere than the pinned directories: they end here.
    if (chain.length <= level || pinned.some((at, depth) => chain[depth] !== at)) {
      break
    }
    const at = chain[level]!
    // Still below its slot: no pin here, nor for the directories inside it. At the
    // slot exactly, the pinned copy sits where the row is, so the swap shows nothing.
    if (topOf(at) > slotTop) {
      break
    }
    // Nothing of it below the slot (a file, a closed or empty directory): the directory is past.
    const last = index.end[at]!
    const bottom = last > at ? topOf(last) + rowPx : 0
    if (bottom <= slotTop) {
      break
    }
    pinned.push(at)
    paths.push(rows[at]!.path)
    offset = Math.min(0, bottom - (slotTop + pitch))
  }
  return { paths, offset }
}

/** What a key does in a tree: move the keyboard cursor to a row, or activate a row as a click would. */
export type TreeKeyEffect = { kind: 'cursor'; path: string } | { kind: 'activate'; path: string }

/**
 * A key pressed in a tree whose keyboard cursor is on `cursor` (null for none
 * yet), as WAI-ARIA's tree does it:
 *
 * - Up and Down move the cursor a row, from the first row when there is none.
 * - Home and End move it to the first and the last.
 * - Right unfolds a folded directory.
 * - Left folds an unfolded one, and from any other row goes to its directory.
 * - Enter activates the cursor's row.
 *
 * Null for a key the tree leaves alone.
 */
export function treeKeyEffect(rows: readonly TreeRow[], cursor: string | null, key: string): TreeKeyEffect | null {
  if (rows.length === 0)
    return null
  const index = cursor === null ? -1 : rows.findIndex((row) => row.path === cursor)
  const row = index < 0 ? null : rows[index]!
  const to = (at: number): TreeKeyEffect => ({ kind: 'cursor', path: rows[at]!.path })
  switch (key) {
    case 'ArrowDown':
      return to(Math.min(rows.length - 1, index + 1))
    case 'ArrowUp':
      return to(Math.max(0, index - 1))
    case 'Home':
      return to(0)
    case 'End':
      return to(rows.length - 1)
    case 'ArrowRight':
      return row?.isDirectory && !row.open ? { kind: 'activate', path: row.path } : null
    case 'ArrowLeft':
      if (!row)
        return null
      if (row.isDirectory && row.open)
        return { kind: 'activate', path: row.path }
      return row.parent !== null && rows.some((entry) => entry.path === row.parent)
        ? { kind: 'cursor', path: row.parent }
        : null
    case 'Enter':
      return row ? { kind: 'activate', path: row.path } : null
    default:
      return null
  }
}
