/** The file view's tree width in px: what it opens at, and how far its divider lets it go. */
export const TREE_WIDTH = {
  min: 120,
  max: 480,
  default: 220,
} as const

/** How long the tree takes to show or hide, docked or over the view. */
export const TREE_MOTION_MS = 200

/**
 * The least the view keeps beside a tree that shows on its own, in px: about
 * 45 columns of code with its line numbers, enough to read a file beside the
 * tree. A narrower frame hides the tree until the user shows it.
 */
export const SHOWN_CONTENT_MIN_WIDTH = 400

/**
 * The least the view keeps beside a docked tree, in px: a few words of a line.
 * Only a narrower frame, such as a window at its smallest, shows the tree over
 * the view instead.
 */
export const DOCKED_CONTENT_MIN_WIDTH = 240

/** The commit list's height under the Change view's files, in px: what it opens at, and the least each pane keeps. */
export const COMMITS_HEIGHT = {
  min: 96,
  default: 240,
} as const
