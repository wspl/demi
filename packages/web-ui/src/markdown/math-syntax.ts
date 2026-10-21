// The syntax of math in Markdown, with nothing to load: `$$…$$` is display math,
// and `$…$` is inline math by Pandoc's rule, so prices such as `$10 vs $12`
// stay text while math may touch Chinese text, as in `面积为$x^2$平方米`.
// `math.ts` draws it; what reads Markdown's own delimiters in text, as the
// streaming reply's closers do (`ui/stream-reveal.ts`), reads it here.
import type { Tokens } from 'marked'

/** `$$…$$` within a line: display math. */
const INLINE_DISPLAY_MATH = /^\$\$(?!\$)((?:\\.|[^\\\n])*?(?:\\.|[^\\\n$]))\$\$/

/**
 * `$…$`: the opening `$` has no space after it, the first `$` that follows
 * closes, and it has no space before it and no digit after it.
 */
const INLINE_MATH = /^\$(?![\s$])((?:\\[^]|[^\\$])*?(?:\\[^]|[^\s\\$]))\$(?!\d)/

/** Where math could start in inline text, so the text before it stops there. */
export function inlineMathStart(src: string): number | undefined {
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
export function inlineMathToken(src: string): Tokens.Generic | undefined {
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
export function blockMathToken(src: string): Tokens.Generic | undefined {
  const match = BLOCK_MATH.exec(src)
  if (!match)
    return undefined
  const [raw, delimiter = '', tex = ''] = match
  return { type: 'blockMath', raw, text: tex.trim(), displayMode: delimiter.length === 2 }
}

/**
 * `text` with every piece of math in it replaced by a placeholder character
 * that is no Markdown, as the Markdown renders it: a closed `$…$` or `$$…$$`,
 * a block opened with `$$` or `$` alone on a line and what follows it, and a
 * `$` that opens math not yet closed, as a formula half written while a reply
 * streams. What the math holds, `_` or `*`, is no emphasis, so what reads
 * Markdown's delimiters in text reads this instead.
 */
export function maskMath(text: string): string {
  let masked = ''
  let index = 0
  while (index < text.length) {
    const at = text.slice(index).search(/[$`]/)
    if (at === -1)
      break
    const start = index + at
    masked += text.slice(index, start)
    let length: number
    if (text[start] === '`') {
      // Code is no math: a `$` in it stays, and so does its closing backtick.
      const run = /^`+/.exec(text.slice(start))?.[0] ?? '`'
      const close = text.indexOf(run, start + run.length)
      length = close === -1 ? text.length - start : close + run.length - start
      masked += text.slice(start, start + length)
    } else {
      const mathChars = mathLength(text, start)
      length = Math.max(mathChars, 1)
      masked += mathChars === 0 ? '$' : '\u0000'.repeat(mathChars)
    }
    index = start + length
  }
  return masked + text.slice(index)
}

/** How many characters of `text` the math that starts at its `$` at `at` takes; 0 where no math starts. */
function mathLength(text: string, at: number): number {
  const rest = text.slice(at)
  const block = at === 0 || text[at - 1] === '\n' ? /^(\${1,2})(?:\n|$)/.exec(rest) : null
  if (block) {
    const delimiter = block[1] ?? ''
    const close = rest.indexOf(`\n${delimiter}`, delimiter.length)
    return close === -1 ? rest.length : close + 1 + delimiter.length
  }
  const closed = inlineMathToken(rest)
  if (closed)
    return closed.raw.length
  // A `$` the writer has not closed yet, which no price opens: a digit after it is a price.
  return /^\$[^\s$\d]/.test(rest) ? rest.length : 0
}
