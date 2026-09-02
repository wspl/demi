import type { Builtin } from './io'
import { parseFlags, has } from './flags'
import { SPECS } from './table'
import { openInputs } from './inputs'
import { encodeLatin1 } from '@demicodes/utils'
import { lines } from '../exec/stream'

export const uniq: Builtin = async (ctx) => {
  const flags = parseFlags('uniq', ctx.argv, SPECS.uniq, ctx.line)
  if (flags.operands.length > 2) {
    await ctx.stderr(`uniq: extra operand '${flags.operands[2]}'\nTry 'uniq --help' for more information.\n`)
    return 1
  }
  const { inputs, failed } = await openInputs(ctx, 'uniq', flags.operands.slice(0, 1), undefined, (detail) => `uniq: error reading '-': ${detail}\n`)
  if (failed) return 1
  const counting = has(flags, 'c')
  let previous: string | null = null
  let count = 0
  const emit = async () => {
    if (previous === null) return
    const prefix = counting ? `${String(count).padStart(7)} ` : ''
    await ctx.stdout(encodeLatin1(`${prefix}${previous}\n`))
  }
  for (const input of inputs) {
    for await (const line of lines(input.stream)) {
      if (previous !== null && line.text === previous) {
        count++
      } else {
        await emit()
        previous = line.text
        count = 1
      }
    }
  }
  if (previous !== null) {
    const prefix = counting ? `${String(count).padStart(7)} ` : ''
    await ctx.stdout(encodeLatin1(`${prefix}${previous}\n`))
  }
  return inputs.some((input) => input.readFailed()) ? 1 : 0
}
