import type { Builtin } from './io'
import { encodeLatin1, utf8AsLatin1 } from '@demicodes/utils'
import { interpretEscapes } from './escapes'

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
  let text = utf8AsLatin1(ctx.argv.slice(i).join(' '))
  let stop = false
  if (escapes) ({ text, stop } = interpretEscapes(text, 'echo'))
  await ctx.stdout(encodeLatin1(text + (newline && !stop ? '\n' : '')))
  return 0
}
