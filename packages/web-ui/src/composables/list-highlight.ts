/**
 * The keyboard's highlight in a list: Up and Down Arrow move it one row and
 * wrap at the ends, as a macOS menu's do, and the row it lands on is
 * brought into view. A Menu, a field's suggestions (`path-completion.ts`)
 * and the search window's results share these two.
 */

/**
 * The row the highlight moves to from `index` (-1 for none yet): the first
 * on Down and the last on Up, then one step, wrapping. -1 for an empty list.
 */
export function stepHighlight(index: number, step: 1 | -1, count: number): number {
  if (count <= 0)
    return -1
  if (step > 0)
    return index < count - 1 ? index + 1 : 0
  return index > 0 ? index - 1 : count - 1
}

/** Brings the `index`th row of `root` that `rows` matches into view, scrolling no further than it takes. */
export function revealRow(root: ParentNode | null | undefined, rows: string, index: number): void {
  root?.querySelectorAll(rows)[index]?.scrollIntoView({ block: 'nearest' })
}
