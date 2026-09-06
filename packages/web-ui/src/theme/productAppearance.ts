/** The selected product appearance; gallery presets consume the same axes. */
export const productAppearance = {
  tone: 'ink',
  density: 'regular',
  radius: 'medium',
  shadow: 'hairline',
} as const

/** The tones a user can pick in settings. */
export type ProductTone = 'ink' | 'warm'
export const PRODUCT_TONES: readonly { id: ProductTone; label: string }[] = [
  { id: 'ink', label: 'Ink' },
  { id: 'warm', label: 'Warm' },
]

export type ProductAccent = 'red' | 'orange' | 'yellow' | 'green' | 'cyan' | 'blue' | 'purple'

export interface ProductAccentOption {
  id: ProductAccent
  name: string
  /** The swatch shown in a picker; the theme derives the working colours from the hue. */
  swatch: string
}

/** Seven hues evenly around the wheel, so every common colour has a home. */
export const PRODUCT_ACCENTS: readonly ProductAccentOption[] = [
  { id: 'red', name: 'Red', swatch: 'oklch(60% 0.14 25)' },
  { id: 'orange', name: 'Orange', swatch: 'oklch(60% 0.14 55)' },
  { id: 'yellow', name: 'Yellow', swatch: 'oklch(60% 0.14 95)' },
  { id: 'green', name: 'Green', swatch: 'oklch(60% 0.14 145)' },
  { id: 'cyan', name: 'Cyan', swatch: 'oklch(60% 0.14 200)' },
  { id: 'blue', name: 'Blue', swatch: 'oklch(60% 0.14 250)' },
  { id: 'purple', name: 'Purple', swatch: 'oklch(60% 0.14 300)' },
]

export const DEFAULT_ACCENT: ProductAccent = 'blue'
