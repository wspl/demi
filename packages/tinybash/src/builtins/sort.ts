import type { Builtin } from './io'
import { parseFlags, has, value } from './flags'
import { SPECS } from './table'
import { openInputs } from './inputs'
import { encodeLatin1 } from '@demicodes/utils'
import { lines } from '../exec/stream'
import { tryHelp } from './errors'

function byteCompare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

/** GNU `-n`: leading blanks, an optional sign, digits, an optional fraction; anything else is 0. */
function numericCompare(a: string, b: string): number {
  const na = numericPrefix(a)
  const nb = numericPrefix(b)
  return na < nb ? -1 : na > nb ? 1 : 0
}

function numericPrefix(text: string): number {
  const match = /^[ \t]*(-?)(\d*)(?:\.(\d*))?/.exec(text)!
  const digits = match[2] ?? ''
  const fraction = match[3] ?? ''
  if (digits === '' && fraction === '') return 0
  const n = Number(`${digits || '0'}.${fraction || '0'}`)
  return match[1] === '-' ? -n : n
}

/** The key `-k N` selects: from field N (leading blanks included) to the end of the line. */
function keyFrom(line: string, field: number): string {
  let index = 0
  for (let f = 1; f < field; f++) {
    while (index < line.length && (line[index] === ' ' || line[index] === '\t')) index++
    while (index < line.length && line[index] !== ' ' && line[index] !== '\t') index++
  }
  return line.slice(index)
}

export const sort: Builtin = async (ctx) => {
  const flags = parseFlags('sort', ctx.argv, SPECS.sort, ctx.line)
  const reverse = has(flags, 'r')
  const numeric = has(flags, 'n')
  const unique = has(flags, 'u')
  const keyRaw = value(flags, 'k')
  let field = 0
  if (keyRaw !== undefined) {
    if (!/^[1-9]\d*$/.test(keyRaw)) {
      await ctx.stderr(`sort: ${/^0/.test(keyRaw) ? `invalid number at field start: invalid count at start of '${keyRaw}'` : `invalid number at field start: invalid count at start of '${keyRaw}'`}\n`)
      return 2
    }
    field = Number(keyRaw)
  }
  const { inputs, failed } = await openInputs(ctx, 'sort', flags.operands, (name, detail) => `sort: cannot read: ${name}: ${detail}\n`)
  if (failed) return 2
  const all: string[] = []
  for (const input of inputs) for await (const line of lines(input.stream)) all.push(line.text)
  const keyOf = (line: string) => (field > 0 ? keyFrom(line, field) : line)
  const compareKeys = (a: string, b: string) => (numeric ? numericCompare(keyOf(a), keyOf(b)) : byteCompare(keyOf(a), keyOf(b)))
  const compare = (a: string, b: string) => {
    let result = compareKeys(a, b)
    if (result === 0 && !unique) result = byteCompare(a, b)
    return reverse ? -result : result
  }
  const indexed = all.map((text, index) => ({ text, index }))
  indexed.sort((x, y) => compare(x.text, y.text) || x.index - y.index)
  const out: string[] = []
  for (const item of indexed) {
    if (unique && out.length > 0 && compareKeys(out[out.length - 1]!, item.text) === 0) continue
    out.push(item.text)
  }
  if (out.length > 0) await ctx.stdout(encodeLatin1(`${out.join('\n')}\n`))
  return 0
}

