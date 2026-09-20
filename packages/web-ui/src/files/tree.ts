/** Layout of a tree's rows: the caption and every row are this tall, with a 1px gap. */
export const TREE_ROW_PX = 28
export const TREE_ROW_PITCH_PX = TREE_ROW_PX + 1

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

/** A directory row and the rows it holds, as indices into `rows`: from the row to the last one under it. */
export function treeBlock(rows: readonly TreeRow[], path: string): { first: number; last: number } | null {
  const first = rows.findIndex((row) => row.path === path)
  if (first < 0)
    return null
  const depth = rows[first]!.depth
  let last = first
  while (last + 1 < rows.length && rows[last + 1]!.depth > depth)
    last += 1
  return { first, last }
}

/** What to pin: the directory rows, and how far up the stack rides as the deepest one leaves. */
export interface StickyTreeStack {
  paths: string[]
  /** Zero or negative px: the stack slides up by this much while its deepest directory scrolls out. */
  offset: number
}

/**
 * The directory rows to pin at the top for a viewport scrolled to `scrollTop`.
 * Level by level, the row under the stack's next slot names the chain of
 * directories it sits in (itself included when it is an open directory with
 * rows under it); the chain's directory for that level pins while its own
 * row has scrolled under the slot and its last row has not, so a directory's
 * name stays in sight while the tree scrolls through it and goes once it is
 * past. A row outside the pinned chain ends the stack. As the deepest pinned
 * directory's last row scrolls under the stack, the stack rides up with it
 * instead of vanishing.
 */
export function stickyTreeRows(
  rows: readonly TreeRow[],
  rowTop: (path: string) => number | undefined,
  scrollTop: number,
  captionPx: number,
): StickyTreeStack {
  const byPath = new Map(rows.map((row) => [row.path, row]))
  const chainOf = (row: TreeRow): string[] => {
    const chain = [row.path]
    let parent = row.parent
    while (parent !== null) {
      chain.unshift(parent)
      parent = byPath.get(parent)?.parent ?? null
    }
    return chain
  }
  const isInside = (row: TreeRow, dir: string): boolean => {
    let up = row.parent
    while (up !== null) {
      if (up === dir) {
        return true
      }
      up = byPath.get(up)?.parent ?? null
    }
    return false
  }
  const lastBottom = (dir: string): number => {
    const last = rows.findLast((row) => isInside(row, dir))
    return last ? (rowTop(last.path) ?? 0) + TREE_ROW_PX : 0
  }
  /** The row under `y`: the first one whose bottom is below it. */
  const rowAt = (y: number): TreeRow | undefined =>
    rows.find((row) => {
      const top = rowTop(row.path)
      return top !== undefined && top + TREE_ROW_PX > y
    })
  const paths: string[] = []
  let offset = 0
  for (let level = 0; ; level++) {
    const slotTop = scrollTop + captionPx + level * TREE_ROW_PITCH_PX
    const anchor = rowAt(slotTop)
    if (!anchor) {
      break
    }
    const chain = chainOf(anchor)
    // The row under the slot belongs elsewhere than the pinned directories: they end here.
    if (chain.length <= level || paths.some((path, index) => chain[index] !== path)) {
      break
    }
    const path = chain[level]!
    const top = rowTop(path)
    // Still below its slot: no pin here, nor for the directories inside it. At the
    // slot exactly, the pinned copy sits where the row is, so the swap shows nothing.
    if (top === undefined || top > slotTop) {
      break
    }
    // Nothing of it below the slot (a file, a closed or empty directory): the directory is past.
    const bottom = lastBottom(path)
    if (bottom <= slotTop) {
      break
    }
    paths.push(path)
    offset = Math.min(0, bottom - (slotTop + TREE_ROW_PITCH_PX))
  }
  return { paths, offset }
}
