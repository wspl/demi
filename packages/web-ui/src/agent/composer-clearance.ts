/**
 * The dock veil's fade over the composer's top: the chips row, a chip and the
 * row's step under it (`SessionDock`), a CSS length.
 */
export const COMPOSER_VEIL_FADE = 'var(--spacing-chip) + 0.5rem'

/**
 * Scroll padding above the floating dock: the last block ends clear of the
 * veil's fade even where the chips row does not show, with a step of room
 * besides, so at the end of the transcript the newest line reads at full
 * strength rather than fading into the dock. A CSS length.
 */
export const COMPOSER_CLEARANCE = `${COMPOSER_VEIL_FADE} + 1rem`

/** Right inset so the veil does not paint over the transcript scrollbar. */
export const COMPOSER_VEIL_SCROLLBAR_GUTTER_PX = 16
