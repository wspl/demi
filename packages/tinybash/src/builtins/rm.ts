import type { Builtin } from './io'
import { type FlagSpec, parseFlags, has } from './flags'
import { quoteC, strerror, tryHelp } from './errors'
import { errorCode } from '@demicodes/utils'

export const rmSpec: FlagSpec = { switches: ['r', 'f'], valued: [] }

export const rm: Builtin = async (ctx) => {
  const flags = parseFlags('rm', ctx.argv, rmSpec, ctx.line)
  const force = has(flags, 'f')
  const recursive = has(flags, 'r')
  if (flags.operands.length === 0) {
    if (force) return 0
    await ctx.stderr(`rm: missing operand\n${tryHelp('rm')}`)
    return 1
  }
  let status = 0
  for (const operand of flags.operands) {
    const base = operand.replace(/\/+$/, '').split('/').pop()
    if (base === '.' || base === '..') {
      await ctx.stderr(`rm: refusing to remove '.' or '..' directory: skipping ${quoteC(operand)}\n`)
      status = 1
      continue
    }
    try {
      const stat = await ctx.fs.lstat(operand, { cwd: ctx.cwd })
      if (stat.isDirectory && !recursive) {
        await ctx.stderr(`rm: cannot remove ${quoteC(operand)}: Is a directory\n`)
        status = 1
        continue
      }
      await ctx.fs.rm(operand, { cwd: ctx.cwd, recursive })
    } catch (error) {
      if (force && errorCode(error) === 'ENOENT') continue
      await ctx.stderr(`rm: cannot remove ${quoteC(operand)}: ${strerror(error)}\n`)
      status = 1
    }
  }
  return status
}
