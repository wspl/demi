import { t } from '../locales/text'
import type { TitleText } from '../ui/ui-text'

/** The selected product appearance; gallery presets consume the same axes. */
export const productAppearance = {
  tone: 'ink',
  density: 'regular',
  radius: 'medium',
  shadow: 'hairline',
} as const

/** The tones a user can pick in settings. */
export type ProductTone = 'ink' | 'warm'
export const PRODUCT_TONES: readonly ProductTone[] = ['ink', 'warm']

/** A tone's name, as Settings › General says it. */
export function toneName(tone: ProductTone): TitleText {
  return t(`appearance.tones.${tone}`)
}

export type ProductAccent = 'blue' | 'purple' | 'pink' | 'red' | 'orange' | 'green' | 'teal'

export interface ProductAccentOption {
  id: ProductAccent
  /** The swatch shown in a picker; the theme derives the working colours from the hue. */
  swatch: string
}

export const PRODUCT_ACCENTS: readonly ProductAccentOption[] = [
  { id: 'blue', swatch: 'oklch(65% 0.19 255)' },
  { id: 'purple', swatch: 'oklch(65% 0.19 295)' },
  { id: 'pink', swatch: 'oklch(65% 0.19 350)' },
  { id: 'red', swatch: 'oklch(65% 0.19 25)' },
  { id: 'orange', swatch: 'oklch(65% 0.19 55)' },
  { id: 'green', swatch: 'oklch(65% 0.19 150)' },
  { id: 'teal', swatch: 'oklch(65% 0.19 190)' },
]

/** The accents with their names in the page's language, as a swatch picker lists them. */
export function productAccentOptions(): { id: ProductAccent; name: TitleText; swatch: string }[] {
  return PRODUCT_ACCENTS.map((accent) => ({ ...accent, name: t(`appearance.accents.${accent.id}`) }))
}

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

/**
 * The transcript's text size, in px, when the user has picked none
 * (`product.md` § Following the agent's work). The one default: the page
 * writes it to `--agent-text`, which the stylesheets leave to it.
 */
export const DEFAULT_TRANSCRIPT_TEXT_SIZE = 15

/**
 * The transcript's text size, in px, on the document's `--agent-text`, which
 * the agent's text, the user's messages, the transcript's rows and the
 * composer read.
 */
export function applyTranscriptTextSize(px: number): void {
  document.documentElement.style.setProperty('--agent-text', `${px}px`)
}
