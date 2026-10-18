import { landmarkIcon } from './file-icons'
import { pathSegments } from './paths'
import type { FileBrowserSource } from './types'

/** One crumb of a path bar: a directory on the way to the path, or the path itself. */
export interface Crumb {
  name: string
  path: string
  /** The root's and the home's landmark glyph, which the crumb wears in place of a folder's. */
  icon: string | undefined
}

/**
 * A path bar's crumbs: the whole path, from the Host's root down to the path
 * itself, the root and the home wearing their glyphs (`file-previews.md`
 * § The File view's bars).
 */
export function pathCrumbs(path: string, source: Pick<FileBrowserSource, 'platform' | 'home'>): Crumb[] {
  return pathSegments(path).map((segment) => ({ ...segment, icon: landmarkIcon(segment.path, source) }))
}

/** A crumb's width with its name, and as its glyph alone. */
export interface CrumbWidths {
  full: number
  icon: number
}

export interface CrumbFit {
  /** How many crumbs, from the left, show their glyph alone. */
  collapsed: number
  /**
   * The crumbs the ellipsis item stands for, in their place: `count` of them
   * from `from`. The middle folders go first, from the one after the root;
   * the root only once the last crumb is a glyph too.
   */
  hidden: { from: number; count: number } | null
  /** Even the ellipsis and the last glyph overflow: the bar keeps its right end and loses its left. */
  clipped: boolean
  /** The crumb whose name is cut short with an ellipsis, and the width it is given, glyph included. */
  truncated: { index: number; width: number } | null
}

/** The least room a name keeps when it is cut short; under it, the crumb is a glyph instead. */
const MIN_TRUNCATED_NAME = 40

/**
 * How a path's crumbs fit a bar `available` wide, as Finder's path bar fits
 * a long path: no crumb is ever cut in half at an edge.
 *
 * 1. Crumbs give up their names one at a time from the left, the last one
 *    excepted. When the next to give up is the widest of those still named,
 *    it is cut short with an ellipsis to fill the room left instead, unless
 *    that leaves it less than a few letters.
 * 2. The middle crumbs then go behind one ellipsis item, from the one after
 *    the first, so the first and the last stay whole.
 * 3. The last crumb's name is cut short, then it is a glyph.
 * 4. The first crumb goes behind the ellipsis too. Only when the ellipsis
 *    and the last glyph still overflow is the row clipped.
 */
export function fitCrumbs(crumbs: readonly CrumbWidths[], separator: number, ellipsis: number, available: number): CrumbFit {
  const fits = (fit: Partial<CrumbFit>): CrumbFit => ({ collapsed: 0, hidden: null, clipped: false, truncated: null, ...fit })
  const last = crumbs.length - 1
  if (last < 0)
    return fits({})
  let total = crumbs.reduce((sum, crumb) => sum + crumb.full, 0) + separator * last
  for (let collapsed = 0; collapsed < last; collapsed += 1) {
    if (total <= available)
      return fits({ collapsed })
    let widest = collapsed
    for (let index = collapsed; index < last; index += 1) {
      if (crumbs[index]!.full > crumbs[widest]!.full)
        widest = index
    }
    const crumb = crumbs[collapsed]!
    if (widest === collapsed) {
      const width = Math.floor(crumb.full - (total - available))
      if (width >= crumb.icon + MIN_TRUNCATED_NAME)
        return fits({ collapsed, truncated: { index: collapsed, width } })
    }
    total -= crumb.full - crumb.icon
  }
  if (total <= available)
    return fits({ collapsed: last })
  // The ellipsis and its separator stand for `count` glyphs and as many separators; it
  // stays only where it takes less room than what it stands for.
  let hidden: CrumbFit['hidden'] = null
  for (let count = 1; count < last; count += 1) {
    const shown = total - crumbs.slice(1, 1 + count).reduce((sum, crumb) => sum + crumb.icon + separator, 0) + ellipsis + separator
    if (shown <= available)
      return fits({ collapsed: last, hidden: { from: 1, count } })
    if (count === last - 1 && shown < total) {
      hidden = { from: 1, count }
      total = shown
    }
  }
  const end = crumbs[last]!
  const width = Math.floor(end.full - (total - available))
  if (width >= end.icon + MIN_TRUNCATED_NAME)
    return fits({ collapsed: last, hidden, truncated: { index: last, width } })
  total -= end.full - end.icon
  if (total <= available)
    return fits({ collapsed: crumbs.length, hidden })
  if (last === 0)
    return fits({ collapsed: 1, clipped: true })
  // Only the ellipsis and the last glyph are left.
  total = ellipsis + separator + end.icon
  return fits({ collapsed: crumbs.length, hidden: { from: 0, count: last }, clipped: total > available })
}
