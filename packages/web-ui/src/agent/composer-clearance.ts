/**
 * How far the dock veil fades above the composer's top, over the chips row and
 * the transcript above it.
 */
export const COMPOSER_VEIL_FADE_PX = 96

/**
 * The veil's opacity along its fade, from the composer's top (0) to the fade's
 * end (1): an ease-in-out cubic, so the fade has no edge at either end.
 */
export function composerVeilOpacity(t: number): number {
  return 1 - (t < 0.5 ? 4 * t ** 3 : 1 - (2 - 2 * t) ** 3 / 2)
}

/**
 * Scroll padding above what the dock veils: at the end of the transcript the
 * newest line ends a step clear of the fade and reads at full strength.
 */
export const COMPOSER_CLEARANCE_PX = 16

/** Right inset so the veil does not paint over the transcript scrollbar. */
export const COMPOSER_VEIL_SCROLLBAR_GUTTER_PX = 16
