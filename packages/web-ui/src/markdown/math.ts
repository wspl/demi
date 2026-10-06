// The math syntax of every Markdown render, in a message and in a file
// (`file-previews.md` § Markdown): `$$…$$` is display math, and `$…$` is
// inline math by Pandoc's rule, so prices such as `$10 vs $12` stay text
// while math may touch Chinese text, as in `面积为$x^2$平方米`.
import type { TokenizerAndRendererExtension, Tokens } from 'marked'
import markedKatex, { type MarkedKatexOptions } from 'marked-katex-extension'

/** `$$…$$` within a line: display math, as marked-katex-extension reads it. */
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

/** The inline math token at the start of `src`, in the extension's token shape. */
function inlineMathToken(type: string, src: string): Tokens.Generic | undefined {
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
 * marked-katex-extension's block math and KaTeX rendering, with Pandoc's
 * inline rule. The extension offers only two inline rules: its standard one
 * wants a space before the opening `$`, which math beside Chinese text lacks,
 * and its `nonStandard` one takes any pair of dollar signs as math, prices
 * included. Neither is Pandoc's, so its inline tokenizer is replaced here
 * and its renderer kept.
 */
export function mathExtensions(options: MarkedKatexOptions): TokenizerAndRendererExtension[] {
  return (markedKatex(options).extensions ?? []).map((extension) => {
    if (!('tokenizer' in extension) || extension.level !== 'inline')
      return extension
    return {
      ...extension,
      start: inlineMathStart,
      tokenizer: (src: string) => inlineMathToken(extension.name, src),
    }
  })
}
