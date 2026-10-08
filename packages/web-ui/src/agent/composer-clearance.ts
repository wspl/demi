/** How far the dock veil extends into the transcript above the dock's top, whichever part of the dock stands highest. */
export const COMPOSER_VEIL_FADE_PX = 32

/**
 * Scroll padding above the floating dock: the last block ends clear of the
 * veil's fade, with a step of room besides, so at the end of the transcript
 * the newest line reads at full strength rather than fading into the dock.
 */
export const COMPOSER_CLEARANCE_PX = COMPOSER_VEIL_FADE_PX + 8

/** Right inset so the veil does not paint over the transcript scrollbar. */
export const COMPOSER_VEIL_SCROLLBAR_GUTTER_PX = 16
