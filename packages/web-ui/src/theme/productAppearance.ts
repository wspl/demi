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

export type ProductAccent = 'steel' | 'indigo' | 'teal' | 'moss' | 'amber' | 'coral' | 'violet'

export interface ProductAccentOption {
  id: ProductAccent
  name: string
  /** The swatch shown in a picker; the theme derives the working colours from the hue. */
  swatch: string
}

export const PRODUCT_ACCENTS: readonly ProductAccentOption[] = [
  { id: 'steel', name: 'Steel', swatch: 'oklch(58% 0.11 250)' },
  { id: 'indigo', name: 'Indigo', swatch: 'oklch(56% 0.12 285)' },
  { id: 'teal', name: 'Teal', swatch: 'oklch(54% 0.09 200)' },
  { id: 'moss', name: 'Moss', swatch: 'oklch(52% 0.09 145)' },
  { id: 'amber', name: 'Amber', swatch: 'oklch(62% 0.11 75)' },
  { id: 'coral', name: 'Coral', swatch: 'oklch(58% 0.11 28)' },
  { id: 'violet', name: 'Violet', swatch: 'oklch(56% 0.12 305)' },
]
