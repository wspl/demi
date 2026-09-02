import type { Builtin } from './io'
import { parseFlags, has } from './flags'
import { SPECS } from './table'
import { openInputs } from './inputs'
import { encodeLatin1 } from '@demicodes/utils'
import { lines } from '../exec/stream'
import { outside } from '../outside/reasons'

type Range = { from: number; to: number | 'last' }

/** `Np`, `N,Mp`, `$p`, `N,$p` — printing line ranges, nothing else. */
function parseProgram(program: string, line: number): Range {
  const match = /^(\d+|\$)(?:,(\d+|\$))?p$/.exec(program.trim())
  if (!match) {
    const flag = program.startsWith('s') ? program : program
    outside({ kind: 'flag', program: 'sed', flag, line })
  }
  const from = match[1] === '$' ? -1 : Number(match[1])
  const to: number | 'last' | undefined = match[2] === undefined ? undefined : match[2] === '$' ? 'last' : Number(match[2])
  if (from === -1) return { from: -1, to: to ?? 'last' }
  return { from, to: to ?? from }
}

/** The operands after the script are files; `-n` is required, `-i` and `s///` are refused. */
export function sedPaths(argv: readonly string[], line: number): string[] {
  const flags = parseFlags('sed', argv, SPECS.sed, line)
  if (!has(flags, 'n')) outside({ kind: 'flag', program: 'sed', flag: 'without -n', line })
  if (flags.operands.length === 0) return []
  parseProgram(flags.operands[0]!, line)
  return flags.operands.slice(1).filter((operand) => operand !== '-')
}

export const sed: Builtin = async (ctx) => {
  const flags = parseFlags('sed', ctx.argv, SPECS.sed, ctx.line)
  if (flags.operands.length === 0) {
    await ctx.stderr(`Usage: sed [OPTION]... {script-only-if-no-other-script} [input-file]...\n`)
    return 1
  }
  const range = parseProgram(flags.operands[0]!, ctx.line)
  const { inputs, failed } = await openInputs(ctx, 'sed', flags.operands.slice(1))
  if (inputs.length === 0) return failed ? 2 : 0
  if (range.from === 0 && range.to === 0) {
    await ctx.stderr(`sed: -e expression #1, char ${flags.operands[0]!.length}: invalid usage of line address 0\n`)
    return 1
  }
  // All inputs form one stream, so `$` is the last line of the last file.
  const all: { text: string; newline: boolean }[] = []
  for (const input of inputs) for await (const line of lines(input.stream)) all.push(line)
  const last = all.length
  const from = range.from === -1 ? last : range.from
  const to = range.to === 'last' ? last : range.to
  for (let n = 1; n <= all.length; n++) {
    const selected = n === from || (n > from && n <= to)
    if (!selected) continue
    const line = all[n - 1]!
    await ctx.stdout(encodeLatin1(`${line.text}${line.newline ? '\n' : ''}`))
  }
  return failed ? 2 : 0
}
