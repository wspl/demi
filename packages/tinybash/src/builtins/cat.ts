import type { Builtin } from './io'
import { parseFlags, has } from './flags'
import { SPECS } from './table'
import { lazyInputs } from './inputs'
import { encodeLatin1 } from '@demicodes/utils'
import { lines } from '../exec/stream'

export const cat: Builtin = async (ctx) => {
  const flags = parseFlags('cat', ctx.argv, SPECS.cat, ctx.line)
  let status = 0
  let number = 1
  for (const input of lazyInputs(ctx, 'cat', flags.operands)) {
    const stream = await input.open()
    if (stream === null) {
      status = 1
      continue
    }
    if (!has(flags, 'n')) {
      for await (const chunk of stream) await ctx.stdout(chunk)
      continue
    }
    for await (const line of lines(stream)) {
      await ctx.stdout(encodeLatin1(`${String(number++).padStart(6)}\t${line.text}${line.newline ? '\n' : ''}`))
    }
  }
  return status
}
