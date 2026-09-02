import type { Builtin } from './io'
import { parseFlags } from './flags'
import { SPECS } from './table'

export const pwd: Builtin = async (ctx) => {
  parseFlags('pwd', ctx.argv, SPECS.pwd, ctx.line)
  await ctx.stdout(`${ctx.cwd}\n`)
  return 0
}

export const trueBuiltin: Builtin = async () => 0
export const falseBuiltin: Builtin = async () => 1
