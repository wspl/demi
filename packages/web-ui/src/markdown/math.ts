// The math syntax of every Markdown render, in a message and in a file: `$$…$$`
// is display math, and `$…$` is inline math by Pandoc's rule, so prices such as
// `$10 vs $12` stay text while math may touch Chinese text, as in
// `面积为$x^2$平方米`.
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

/** `$$…$$` within a line: display math. */
const INLINE_DISPLAY_MATH = /^\$\$(?!\$)((?:\\.|[^\\\n])*?(?:\\.|[^\\\n$]))\$\$/

/**
 * `$…$`: the opening `$` has no space after it, the first `$` that follows
 * closes, and it has no space before it and no digit after it.
 */
const INLINE_MATH = /^\$(?![\s$])((?:\\[^]|[^\\$])*?(?:\\[^]|[^\s\\$]))\$(?!\d)/

/** Where math could start in inline text, so the text before it stops there. */
function inlineMathStart(src: string): number | undefined {
  let index = src.indexOf('$')
  while (index !== -1) {
    const rest = src.slice(index)
    if (INLINE_DISPLAY_MATH.test(rest) || INLINE_MATH.test(rest))
      return index
    // A run of dollar signs that starts no math is text as a whole.
    const after = rest.search(/[^$]/)
    if (after === -1)
      return undefined
    index = src.indexOf('$', index + after)
  }
  return undefined
}

/** The inline math token at the start of `src`. */
function inlineMathToken(src: string): Tokens.Generic | undefined {
  const type = 'inlineMath'
  const display = INLINE_DISPLAY_MATH.exec(src)
  if (display) {
    const [raw, tex = ''] = display
    return { type, raw, text: tex.trim(), displayMode: true }
  }
  const inline = INLINE_MATH.exec(src)
  if (inline) {
    const [raw, tex = ''] = inline
    return { type, raw, text: tex.trim(), displayMode: false }
  }
  return undefined
}

/**
 * `$$` or `$` alone on a line, the TeX, and the same alone on a line: block
 * math, display for `$$`.
 */
const BLOCK_MATH = /^(\${1,2})\n((?:\\[^]|[^\\])+?)\n\1(?:\n|$)/

/** The block math token at the start of `src`. */
function blockMathToken(src: string): Tokens.Generic | undefined {
  const match = BLOCK_MATH.exec(src)
  if (!match)
    return undefined
  const [raw, delimiter = '', tex = ''] = match
  return { type: 'blockMath', raw, text: tex.trim(), displayMode: delimiter.length === 2 }
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
