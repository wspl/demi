import { clampSize, type ResizeBounds } from '../ui/resize-handle'

/** The sidebar's width in px: what it opens at, and how far the divider lets it go. */
export const SIDEBAR_WIDTH = {
  min: 200,
  max: 420,
  default: 256,
} as const

/**
 * The work panel is sized as a share of the width it splits with the
 * conversation (the frame minus the sidebar), so resizing the window scales
 * both sides together while the sidebar keeps its px width. The px floors
 * keep either side usable: the panel never goes under `minWidth`, and it
 * leaves the conversation at least `mainMinWidth` while there is room for both.
 */
export const ASIDE_SHARE = {
  default: 0.4,
  minWidth: 280,
  mainMinWidth: 360,
} as const

/** The px bounds the work panel's divider moves within, for a given shared width. */
export function asideBounds(sharedWidth: number): ResizeBounds {
  return {
    min: ASIDE_SHARE.minWidth,
    max: Math.max(ASIDE_SHARE.minWidth, Math.round(sharedWidth - ASIDE_SHARE.mainMinWidth)),
  }
}

/** The work panel's px width for its share of the width it splits with the conversation. */
export function asideWidthFor(share: number, sharedWidth: number): number {
  return clampSize(share * sharedWidth, asideBounds(sharedWidth))
}

/** The share a px width is of the shared width; what a drag stores. */
export function asideShareFor(width: number, sharedWidth: number): number {
  return sharedWidth > 0 ? width / sharedWidth : ASIDE_SHARE.default
}
