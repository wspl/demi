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

/**
 * The directory rows to pin at the top for a viewport scrolled to `scrollTop`:
 * the directories that enclose the selected file, each pinned once its own
 * row has scrolled under the stack above it, so the selected file's path
 * stays in sight while the rest of the tree scrolls freely. Other unfolded
 * directories never pin, so the stack changes only with the scroll position
 * of these few rows.
 */
export function stickyTreeRows(
  rows: readonly FileTreeRow[],
  rowTop: (path: string) => number | undefined,
  scrollTop: number,
  captionPx: number,
  selected: string | null,
): string[] {
  const byPath = new Map(rows.map((row) => [row.path, row]))
  const selectedRow = selected === null ? undefined : byPath.get(selected)
  if (!selectedRow) {
    return []
  }
  const ancestors: string[] = []
  let parent = selectedRow.parent
  while (parent !== null) {
    ancestors.unshift(parent)
    parent = byPath.get(parent)?.parent ?? null
  }
  const paths: string[] = []
  for (const [level, path] of ancestors.entries()) {
    const top = rowTop(path)
    // A directory whose row is still visible below the stack needs no pin, nor do those under it.
    if (top === undefined || top >= scrollTop + captionPx + level * TREE_ROW_PITCH_PX) {
      break
    }
    paths.push(path)
  }
  return paths
}
