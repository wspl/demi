import type { Builtin } from './io'
import { parseFlags } from './flags'
import { SPECS } from './table'
import { quoteC, strerror, tryHelp } from './errors'

export const touch: Builtin = async (ctx) => {
  const flags = parseFlags('touch', ctx.argv, SPECS.touch, ctx.line)
  if (flags.operands.length === 0) {
    await ctx.stderr(`touch: missing file operand\n${tryHelp('touch')}`)
    return 1
  }
  let status = 0
  const now = new Date()
  for (const operand of flags.operands) {
    try {
      if (await ctx.fs.exists(operand, { cwd: ctx.cwd })) await ctx.fs.utimes(operand, now, now, { cwd: ctx.cwd })
      else await ctx.fs.writeFile(operand, new Uint8Array(0), { cwd: ctx.cwd })
    } catch (error) {
      await ctx.stderr(`touch: cannot touch ${quoteC(operand)}: ${strerror(error)}\n`)
      status = 1
    }
  }
  return status
}
