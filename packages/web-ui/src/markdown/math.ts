// The math of every Markdown render, in a message and in a file, drawn by KaTeX
// over the syntax of `math-syntax.ts`.
//
// KaTeX is a chunk of its own, which the page loads in the background once the
// first conversation has shown (`infra/region-load.ts`), with its fonts, and
// math shown starts loading it if it is not there yet. Math is a region
// (`region-wait.ts`) that needs KaTeX and every font of its stylesheet, since a
// formula drawn without a font it needs shows no glyphs for it: for 300 ms at
// most, in all, math draws nothing yet, as its TeX source hidden where it will
// stand; past that, the source, which stays until the formula replaces it. A
// formula's height is not known until it renders, so what is not shown yet
// after a hidden formula waits with it (`.math-waiting` in `base.css`); what
// already shows does not. Every Markdown render runs again on KaTeX's arrival.
import type { KatexOptions } from 'katex'
import type { TokenizerAndRendererExtension, Tokens } from 'marked'
import { chunkLoadFailure, reportError } from '../infra/errors'
import { t } from '../locales/text'
import { regionForm, regionsChanged, type RegionForm } from '../infra/region-wait'
import { escapeHtml } from './html'
import { blockMathToken, inlineMathStart, inlineMathToken } from './math-syntax'
import { renderAgain } from './render-version'

/** KaTeX, from its chunk. */
export type Katex = typeof import('./katex').default

let katex: Katex | null = null
let loading: Promise<void> | null = null

/**
 * Loads every font of KaTeX's stylesheet. A browser fetches a font once text
 * uses it, and a formula that needs one it has not got, such as the size
 * faces of a root or a large operator, shows no glyphs for it until it
 * arrives. A font that fails to load is the stylesheet's fallback.
 */
async function loadKatexFonts(): Promise<void> {
  const fonts = typeof document === 'undefined' ? undefined : document.fonts
  if (!fonts)
    return
  await Promise.allSettled([...fonts].filter((face) => face.family.includes('KaTeX')).map((face) => face.load()))
}

/**
 * Loads KaTeX and its fonts once; Markdown renders again when both are there.
 * One that fails to load leaves math as its source, as a highlighter that
 * fails to load leaves code plain, and says so once.
 */
export function loadKatex(): Promise<void> {
  loading ??= import('./katex').then(
    async (chunk) => {
      await loadKatexFonts()
      katex = chunk.default
      regionsChanged()
      renderAgain()
    },
    (error: unknown) => reportError(t('markdown.mathUnavailable'), chunkLoadFailure(error)),
  )
  return loading
}

/** KaTeX, null until it has loaded: the first call starts its load. */
export function loadedKatex(): Katex | null {
  if (!katex)
    void loadKatex()
  return katex
}

/**
 * How math draws now: `final` once KaTeX and its fonts are in, which `katex`
 * answers.
 */
export function mathForm(katex: () => Katex | null = loadedKatex): RegionForm {
  return regionForm('math', () => katex() !== null, renderAgain)
}

/**
 * Math as it shows until KaTeX is there: its TeX source, in a display of its
 * own line or inline, hidden while `waiting`. A span in both cases, since
 * display math may stand within a paragraph, where a block element would end
 * it.
 */
export function mathSourceHtml(tex: string, displayMode: boolean, waiting = false): string {
  return `<span class="math ${displayMode ? 'math-display' : 'math-inline'}${waiting ? ' math-waiting' : ''}">${escapeHtml(tex)}</span>`
}

/**
 * Math in Markdown, by Pandoc's inline rule, rendered by KaTeX with `options`
 * once it has loaded, and until then as its source, hidden for the first
 * 300 ms.
 *
 * The inline rule is Pandoc's, not marked-katex-extension's, for three reasons:
 *
 * - Its standard rule wants a space before the opening `$`, which math beside
 *   Chinese text lacks.
 * - Its `nonStandard` rule takes any pair of dollar signs as math, prices
 *   included.
 * - It imports KaTeX, which must stay out of the page until math shows.
 *
 * `katex` answers KaTeX once it has loaded, the page's own load unless a test
 * stands in for it.
 */
export function mathExtensions(options: KatexOptions, katex: () => Katex | null = loadedKatex): TokenizerAndRendererExtension[] {
  const render = (token: Tokens.Generic): string => {
    const tex = String(token['text'])
    const displayMode = token['displayMode'] === true
    const form = mathForm(katex)
    const drawn = form === 'final' ? katex()?.renderToString(tex, { ...options, displayMode }) : undefined
    return drawn ?? mathSourceHtml(tex, displayMode, form === 'held')
  }
  return [
    {
      name: 'inlineMath',
      level: 'inline',
      start: inlineMathStart,
      tokenizer: inlineMathToken,
      renderer: render,
    },
    {
      name: 'blockMath',
      level: 'block',
      tokenizer: blockMathToken,
      renderer: (token) => `${render(token)}\n`,
    },
  ]
}
