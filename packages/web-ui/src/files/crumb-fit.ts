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
  /** Even every glyph overflows: the bar keeps its right end and loses its left. */
  clipped: boolean
}

/**
 * How a path's crumbs fit a bar `available` wide. Crumbs give up their names
 * one at a time from the left, so those nearest the end keep theirs longest;
 * only when every crumb is a glyph and the row still overflows is it clipped.
 */
export function fitCrumbs(crumbs: readonly CrumbWidths[], separator: number, available: number): CrumbFit {
  let total = crumbs.reduce((sum, crumb) => sum + crumb.full, 0) + separator * Math.max(0, crumbs.length - 1)
  for (let collapsed = 0; collapsed < crumbs.length; collapsed += 1) {
    if (total <= available)
      return { collapsed, clipped: false }
    total -= crumbs[collapsed]!.full - crumbs[collapsed]!.icon
  }
  return { collapsed: crumbs.length, clipped: total > available }
}
