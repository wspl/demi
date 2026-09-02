import type { Builtin, BuiltinContext } from './io'
import { type ParsedFlags, parseFlags, value } from './flags'
import { SPECS } from './table'
import { lazyInputs } from './inputs'
import { collectBytes, decodeLatin1, encodeLatin1 } from '@demicodes/utils'
import { outside } from '../outside/reasons'

function parseCount(raw: string): number | null {
  if (!/^[0-9]+$/.test(raw)) return null
  return Number(raw)
}

async function eachInput(ctx: BuiltinContext, program: string, operands: readonly string[], emit: (bytes: Uint8Array) => Promise<void>): Promise<number> {
  const inputs = lazyInputs(ctx, program, operands, (name, detail) => `${program}: cannot open '${name}' for reading: ${detail}\n`)
  const headers = inputs.length > 1
  let status = 0
  let first = true
  for (const input of inputs) {
    const stream = await input.open()
    if (stream === null) {
      status = 1
      continue
    }
    if (headers) {
      await ctx.stdout(`${first ? '' : '\n'}==> ${input.name} <==\n`)
      first = false
    }
    await emit(await collectBytes(stream))
  }
  return status
}

/** Splits into lines keeping each newline attached, as GNU counts them. */
function splitKeepNewlines(text: string): string[] {
  const out: string[] = []
  let start = 0
  for (;;) {
    const index = text.indexOf('\n', start)
    if (index === -1) break
    out.push(text.slice(start, index + 1))
    start = index + 1
  }
  if (start < text.length) out.push(text.slice(start))
  return out
}

/** The obsolete `-N` spelling GNU still accepts, rewritten to `-n N`. */
function obsoleteCount(argv: readonly string[]): string[] {
  const out: string[] = []
  let expectingValue = false
  for (const arg of argv) {
    if (!expectingValue && /^-[0-9]+$/.test(arg)) out.push('-n', arg.slice(1))
    else out.push(arg)
    expectingValue = !expectingValue && (arg === '-n' || arg === '-c')
  }
  return out
}

/** Signed counts (`-n -5`, `tail -c +3`) are GNU forms outside the whitelist. */
function parseCounted(program: 'head' | 'tail', argv: readonly string[], line: number): ParsedFlags {
  const flags = parseFlags(program, obsoleteCount(argv), SPECS[program], line)
  const n = value(flags, 'n')
  const c = value(flags, 'c')
  if (n !== undefined && (n.startsWith('-') || (program === 'head' && n.startsWith('+')))) outside({ kind: 'flag', program, flag: `-n ${n}`, line })
  if (c !== undefined && /^[+-]/.test(c)) outside({ kind: 'flag', program, flag: `-c ${c}`, line })
  return flags
}

export const head: Builtin = async (ctx) => {
  const flags = parseCounted('head', ctx.argv, ctx.line)
  const n = value(flags, 'n')
  const c = value(flags, 'c')
  const count = c !== undefined ? parseCount(c) : parseCount(n ?? '10')
  if (count === null) {
    await ctx.stderr(`head: invalid number of ${c !== undefined ? 'bytes' : 'lines'}: '${c ?? n}'\n`)
    return 1
  }
  return eachInput(ctx, 'head', flags.operands, async (bytes) => {
    if (c !== undefined) {
      await ctx.stdout(bytes.subarray(0, count))
      return
    }
    const parts = splitKeepNewlines(decodeLatin1(bytes)).slice(0, count)
    if (parts.length > 0) await ctx.stdout(encodeLatin1(parts.join('')))
  })
}

export const tail: Builtin = async (ctx) => {
  const flags = parseCounted('tail', ctx.argv, ctx.line)
  const n = value(flags, 'n')
  const c = value(flags, 'c')
  const fromStart = n !== undefined && n.startsWith('+')
  const raw = c ?? (fromStart ? n!.slice(1) : n ?? '10')
  const count = parseCount(raw)
  if (count === null) {
    await ctx.stderr(`tail: invalid number of ${c !== undefined ? 'bytes' : 'lines'}: '${c ?? n}'\n`)
    return 1
  }
  return eachInput(ctx, 'tail', flags.operands, async (bytes) => {
    if (c !== undefined) {
      await ctx.stdout(bytes.subarray(Math.max(0, bytes.length - count)))
      return
    }
    const parts = splitKeepNewlines(decodeLatin1(bytes))
    const selected = fromStart ? parts.slice(Math.max(0, count - 1)) : count === 0 ? [] : parts.slice(Math.max(0, parts.length - count))
    if (selected.length > 0) await ctx.stdout(encodeLatin1(selected.join('')))
  })
}

export function headTailPaths(program: 'head' | 'tail'): (argv: readonly string[], line: number) => string[] {
  return (argv, line) => parseCounted(program, argv, line).operands.filter((operand) => operand !== '-')
}
