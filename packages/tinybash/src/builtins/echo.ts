import type { Builtin } from './io'

/** bash's `echo`: `-n` and `-e` (and their combinations) are options; anything else is text. */
export const echo: Builtin = async (ctx) => {
  let newline = true
  let escapes = false
  let i = 0
  for (; i < ctx.argv.length; i++) {
    const arg = ctx.argv[i]!
    if (!/^-[neE]+$/.test(arg)) break
    for (const ch of arg.slice(1)) {
      if (ch === 'n') newline = false
      else if (ch === 'e') escapes = true
      else escapes = false
    }
  }
  let text = ctx.argv.slice(i).join(' ')
  let stop = false
  if (escapes) ({ text, stop } = interpretEscapes(text))
  await ctx.stdout(text + (newline && !stop ? '\n' : ''))
  return 0
}

/** The escapes `echo -e` knows; `\c` ends output. */
export function interpretEscapes(text: string): { text: string; stop: boolean } {
  let out = ''
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]!
    if (ch !== '\\' || i + 1 >= text.length) {
      out += ch
      continue
    }
    const next = text[i + 1]!
    i++
    switch (next) {
      case 'a': out += '\x07'; break
      case 'b': out += '\b'; break
      case 'c': return { text: out, stop: true }
      case 'e': case 'E': out += '\x1b'; break
      case 'f': out += '\f'; break
      case 'n': out += '\n'; break
      case 'r': out += '\r'; break
      case 't': out += '\t'; break
      case 'v': out += '\v'; break
      case '\\': out += '\\'; break
      case '0': {
        const digits = /^[0-7]{1,3}/.exec(text.slice(i + 1))?.[0] ?? ''
        out += String.fromCharCode(parseInt(digits || '0', 8))
        i += digits.length
        break
      }
      case 'x': {
        const digits = /^[0-9A-Fa-f]{1,2}/.exec(text.slice(i + 1))?.[0]
        if (digits === undefined) {
          out += '\\x'
          break
        }
        out += String.fromCharCode(parseInt(digits, 16))
        i += digits.length
        break
      }
      default:
        out += `\\${next}`
    }
  }
  return { text: out, stop: false }
}
