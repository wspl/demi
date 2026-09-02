import type { Builtin } from './io'
import { type FlagSpec, parseFlags, has } from './flags'
import { quoteC, strerror, tryHelp } from './errors'

export const mkdirSpec: FlagSpec = { switches: ['p'], valued: [] }

export const mkdir: Builtin = async (ctx) => {
  const flags = parseFlags('mkdir', ctx.argv, mkdirSpec, ctx.line)
  if (flags.operands.length === 0) {
    await ctx.stderr(`mkdir: missing operand\n${tryHelp('mkdir')}`)
    return 1
  }
  const parents = has(flags, 'p')
  let status = 0
  for (const operand of flags.operands) {
    try {
      if (parents) {
        const existing = await ctx.fs.stat(operand, { cwd: ctx.cwd }).catch(() => null)
        if (existing !== null) {
          if (existing.isDirectory) continue
          await ctx.stderr(`mkdir: cannot create directory ${quoteC(operand)}: File exists\n`)
          status = 1
          continue
        }
      }
      await ctx.fs.mkdir(operand, { cwd: ctx.cwd, recursive: parents })
    } catch (error) {
      await ctx.stderr(`mkdir: cannot create directory ${quoteC(operand)}: ${strerror(error)}\n`)
      status = 1
    }
  }
  return status
}
