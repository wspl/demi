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

/** What to pin: the directory rows, and how far up the stack rides as the deepest one leaves. */
export interface StickyTreeStack {
  paths: string[]
  /** Zero or negative px: the stack slides up by this much while its deepest directory scrolls out. */
  offset: number
}

/**
 * The directory rows to pin at the top for a viewport scrolled to `scrollTop`:
 * the directories that enclose the selected file, each pinned while its own
 * row has scrolled under the stack above it and its last row has not, so the
 * selected file's path stays in sight while the tree scrolls through that
 * directory and goes once the directory is past. Other unfolded directories
 * never pin. As the deepest pinned directory's last row scrolls under the
 * stack, the stack rides up with it instead of vanishing.
 */
export function stickyTreeRows(
  rows: readonly FileTreeRow[],
  rowTop: (path: string) => number | undefined,
  scrollTop: number,
  captionPx: number,
  selected: string | null,
): StickyTreeStack {
  const byPath = new Map(rows.map((row) => [row.path, row]))
  const selectedRow = selected === null ? undefined : byPath.get(selected)
  if (!selectedRow) {
    return { paths: [], offset: 0 }
  }
  const ancestors: string[] = []
  let parent = selectedRow.parent
  while (parent !== null) {
    ancestors.unshift(parent)
    parent = byPath.get(parent)?.parent ?? null
  }
  const isInside = (row: FileTreeRow, dir: string): boolean => {
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
  const paths: string[] = []
  let offset = 0
  for (const [level, path] of ancestors.entries()) {
    const top = rowTop(path)
    const slotTop = scrollTop + captionPx + level * TREE_ROW_PITCH_PX
    // Not yet under the stack: no pin here, nor for the directories inside it.
    if (top === undefined || top >= slotTop) {
      break
    }
    // Its whole subtree is above the slot: the directory is past.
    const bottom = lastBottom(path)
    if (bottom <= slotTop) {
      break
    }
    paths.push(path)
    offset = Math.min(0, bottom - (slotTop + TREE_ROW_PITCH_PX))
  }
  return { paths, offset }
}
