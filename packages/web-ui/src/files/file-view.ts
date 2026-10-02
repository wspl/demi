/** The file view's tree width in px: what it opens at, and how far its divider lets it go. */
export const TREE_WIDTH = {
  min: 120,
  max: 480,
  default: 220,
} as const

/** How long the tree takes to show or hide, docked or over the view. */
export const TREE_MOTION_MS = 200

/** The least the view keeps beside a docked tree, in px; a narrower frame hides the tree instead. */
export const CONTENT_MIN_WIDTH = 320
