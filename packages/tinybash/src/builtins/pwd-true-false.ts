import type { Builtin } from './io'
import { type FlagSpec, parseFlags } from './flags'

export const pwdSpec: FlagSpec = { switches: [], valued: [] }

export const pwd: Builtin = async (ctx) => {
  parseFlags('pwd', ctx.argv, pwdSpec, ctx.line)
  await ctx.stdout(`${ctx.cwd}\n`)
  return 0
}

export const trueBuiltin: Builtin = async () => 0
export const falseBuiltin: Builtin = async () => 1
