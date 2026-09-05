import type { Builtin } from './io'
import { type FlagSpec, parseFlags, has } from './flags'
import { lazyInputs } from './inputs'
import { encodeLatin1 } from '@demicodes/utils'
import { lines } from '../exec/stream'

export const catSpec: FlagSpec = { switches: ['n'], valued: [] }

export const cat: Builtin = async (ctx) => {
  const flags = parseFlags('cat', ctx.argv, catSpec, ctx.line)
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
    } else {
      for await (const line of lines(stream)) {
        await ctx.stdout(encodeLatin1(`${String(number++).padStart(6)}\t${line.text}${line.newline ? '\n' : ''}`))
      }
    }
    if (input.readFailed()) status = 1
  }
  return status
}
