// The math syntax of every Markdown render, in a message and in a file
// (`file-previews.md` § Markdown): `$$…$$` is display math, and `$…$` is
// inline math by Pandoc's rule, so prices such as `$10 vs $12` stay text
// while math may touch Chinese text, as in `面积为$x^2$平方米`.
//
// KaTeX is a chunk of its own, which the first math shown starts loading
// (`file-previews.md` § Markdown). Until it is there, math shows its TeX
// source where the math will stand, inline in its line or a display on a
// line of its own, and every Markdown render runs again on its arrival.
import type { KatexOptions } from 'katex'
import type { TokenizerAndRendererExtension, Tokens } from 'marked'
import { chunkLoadFailure, reportError } from '../infra/errors'
import { t } from '../locales/text'
import { escapeHtml } from './html'
import { renderAgain } from './render-version'

/** KaTeX, from its chunk. */
export type Katex = typeof import('./katex').default

let katex: Katex | null = null
let loading: Promise<void> | null = null

/**
 * Loads KaTeX once; Markdown renders again when it is there. One that fails
 * to load leaves math as its source, as a highlighter that fails to load
 * leaves code plain, and says so once.
 */
export function loadKatex(): Promise<void> {
  loading ??= import('./katex').then(
    (chunk) => {
      katex = chunk.default
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
 * Math as it shows until KaTeX is there: its TeX source, in a display of its
 * own line or inline. A span in both cases, since display math may stand
 * within a paragraph, where a block element would end it.
 */
export function mathSourceHtml(tex: string, displayMode: boolean): string {
  return `<span class="math ${displayMode ? 'math-display' : 'math-inline'}">${escapeHtml(tex)}</span>`
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

/** `$$` or `$` alone on a line, the TeX, and the same alone on a line: block math, display for `$$`. */
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
 * Math in Markdown, by Pandoc's inline rule, rendered by KaTeX with
 * `options` once it has loaded and as its source until then. The inline
 * rule is Pandoc's, not marked-katex-extension's, whose standard rule wants
 * a space before the opening `$`, which math beside Chinese text lacks, and
 * whose `nonStandard` one takes any pair of dollar signs as math, prices
 * included; and that extension imports KaTeX, which must stay out of the
 * page until math shows. `katex` answers KaTeX once it has loaded, the
 * page's own load unless a test stands in for it.
 */
export function mathExtensions(options: KatexOptions, katex: () => Katex | null = loadedKatex): TokenizerAndRendererExtension[] {
  const render = (token: Tokens.Generic): string => {
    const tex = String(token['text'])
    const displayMode = token['displayMode'] === true
    return katex()?.renderToString(tex, { ...options, displayMode }) ?? mathSourceHtml(tex, displayMode)
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
