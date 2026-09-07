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

export type ProductAccent = 'blue' | 'purple' | 'pink' | 'red' | 'orange' | 'green' | 'teal'

export interface ProductAccentOption {
  id: ProductAccent
  name: string
  /** The swatch shown in a picker; the theme derives the working colours from the hue. */
  swatch: string
}

/** Seven hues that work as an accent on a neutral surface; yellows and cyans do not. */
export const PRODUCT_ACCENTS: readonly ProductAccentOption[] = [
  { id: 'blue', name: 'Blue', swatch: 'oklch(65% 0.19 255)' },
  { id: 'purple', name: 'Purple', swatch: 'oklch(65% 0.19 295)' },
  { id: 'pink', name: 'Pink', swatch: 'oklch(65% 0.19 350)' },
  { id: 'red', name: 'Red', swatch: 'oklch(65% 0.19 25)' },
  { id: 'orange', name: 'Orange', swatch: 'oklch(65% 0.19 55)' },
  { id: 'green', name: 'Green', swatch: 'oklch(65% 0.19 150)' },
  { id: 'teal', name: 'Teal', swatch: 'oklch(65% 0.19 190)' },
]

export const DEFAULT_ACCENT: ProductAccent = 'blue'

/** The axes a user changes at runtime; the rest of `productAppearance` is fixed per product. */
export interface ProductAppearanceChoice {
  tone: ProductTone
  accent: ProductAccent
}

/** Puts a choice on the document, where the tokens read it. */
export function applyProductAppearance(choice: ProductAppearanceChoice): void {
  const root = document.documentElement
  root.setAttribute('data-tone', choice.tone)
  root.setAttribute('data-accent', choice.accent)
}

/** The transcript's text size, in px; the composer and previews follow the same token. */
export function applyTranscriptTextSize(px: number): void {
  document.documentElement.style.setProperty('--agent-text', `${px}px`)
}
