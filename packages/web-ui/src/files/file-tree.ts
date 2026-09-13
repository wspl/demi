/** Layout of a file tree's rows: the caption and every row are this tall, with a 1px gap. */
export const TREE_ROW_PX = 28
export const TREE_ROW_PITCH_PX = TREE_ROW_PX + 1

/** One row of the tree as laid out: a file or directory at a depth, under a parent. */
export interface FileTreeRow {
  path: string
  name: string
  isDirectory: boolean
  depth: number
  /** The directory row above it, or null under the root. */
  parent: string | null
}

/** What to pin: the directory rows, and how far up the stack is pushed as the last one leaves. */
export interface StickyTreeStack {
  paths: string[]
  /** Zero or negative px: the stack slides up by this much while its last directory scrolls out. */
  offset: number
}

/**
 * The directory rows to pin at the top for a viewport scrolled to `scrollTop`:
 * the enclosing directories of the rows under the caption, the way an
 * explorer's sticky scroll keeps the path in view. Each pinned row covers one
 * more row of the tree, and the directory pinned at a level is the one at
 * that depth above the row it covers, so a directory whose own row has
 * scrolled under the stack still shows. As the last row of the deepest
 * pinned directory scrolls under the stack, the stack is pushed up with it.
 */
export function stickyTreeRows(
  rows: readonly FileTreeRow[],
  rowTop: (path: string) => number | undefined,
  scrollTop: number,
  captionPx: number,
): StickyTreeStack {
  const byPath = new Map(rows.map((row) => [row.path, row]))
  const ancestorsOf = (row: FileTreeRow): string[] => {
    const chain: string[] = []
    let parent = row.parent
    while (parent !== null) {
      chain.unshift(parent)
      parent = byPath.get(parent)?.parent ?? null
    }
    return chain
  }
  const rowUnder = (y: number): FileTreeRow | undefined =>
    rows.find((row) => {
      const top = rowTop(row.path)
      return top !== undefined && top + TREE_ROW_PX > y
    })
  // Level by level: the row a pinned row at this level would cover names the
  // directory to pin there, as long as it agrees with the levels above.
  const paths: string[] = []
  for (let level = 0; level < 16; level++) {
    const covered = rowUnder(scrollTop + captionPx + level * TREE_ROW_PITCH_PX)
    if (!covered) {
      break
    }
    const ancestors = ancestorsOf(covered)
    const next = ancestors[level]
    if (next === undefined || !paths.every((path, index) => ancestors[index] === path)) {
      break
    }
    paths.push(next)
  }
  const deepest = paths.at(-1)
  if (deepest === undefined) {
    return { paths, offset: 0 }
  }
  const lastInside = rows.findLast((row) => ancestorsOf(row).includes(deepest))
  const lastBottom = lastInside ? (rowTop(lastInside.path) ?? 0) + TREE_ROW_PX : 0
  const stackBottom = scrollTop + captionPx + paths.length * TREE_ROW_PITCH_PX
  return { paths, offset: Math.min(0, lastBottom - stackBottom) }
}
