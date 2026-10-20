/**
 * Where a dragged item of a strip or a list lands, and how the others make
 * room for it, as a web browser's tabs and a macOS list's rows do: the same
 * slots along either axis (`TabStrip`, `useListReorder`).
 */

/** How far the pointer moves before a pressed item starts to drag, so a click with a tremor stays a click. */
export const DRAG_START_PX = 4

/**
 * Where a dragged item lands: its index among the others, the `move` index, by how
 * many of their centers its own center has passed.
 *
 * `centers` are the items' centers in order, the dragged one at `from` included, and
 * `center` is the dragged item's center where the pointer holds it.
 */
export function dragTarget(centers: readonly number[], from: number, center: number): number {
  return centers.filter((other, index) => index !== from && other < center).length
}

/**
 * Which way the item at `index` steps aside while the item at `from` is held
 * over the slot `to` (`dragTarget`): -1 toward the start, 1 toward the end,
 * 0 when it stays.
 */
export function dragShift(index: number, from: number, to: number): -1 | 0 | 1 {
  if (index > from && index - 1 < to) {
    return -1
  }
  if (index < from && index >= to) {
    return 1
  }
  return 0
}

/** `ids` with the one at `from` moved to the slot `to` among the others. */
export function moved<T>(ids: readonly T[], from: number, to: number): T[] {
  const others = ids.filter((_, index) => index !== from)
  others.splice(to, 0, ids[from]!)
  return others
}
