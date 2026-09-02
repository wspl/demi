import type { Builtin } from './io'
import { parseFlags, has } from './flags'
import { SPECS } from './table'
import { collectBytes, utf8AsLatin1 } from '@demicodes/utils'
import { tryHelp } from './errors'
import { guardedStdin } from './inputs'

const CLASSES: Record<string, (c: number) => boolean> = {
  upper: (c) => c >= 65 && c <= 90,
  lower: (c) => c >= 97 && c <= 122,
  digit: (c) => c >= 48 && c <= 57,
  alpha: (c) => (c >= 65 && c <= 90) || (c >= 97 && c <= 122),
  alnum: (c) => (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || (c >= 48 && c <= 57),
  space: (c) => c === 32 || (c >= 9 && c <= 13),
  blank: (c) => c === 32 || c === 9,
  punct: (c) => (c >= 33 && c <= 47) || (c >= 58 && c <= 64) || (c >= 91 && c <= 96) || (c >= 123 && c <= 126),
  print: (c) => c >= 32 && c <= 126,
  graph: (c) => c >= 33 && c <= 126,
  cntrl: (c) => c < 32 || c === 127,
  xdigit: (c) => (c >= 48 && c <= 57) || (c >= 65 && c <= 70) || (c >= 97 && c <= 102),
}

/** A tr SET into its byte values: escapes, ranges and `[:class:]`. */
export function parseSet(set: string): number[] | string {
  const out: number[] = []
  let i = 0
  const readChar = (): number => {
    const ch = set[i]!
    if (ch !== '\\') {
      i++
      return ch.charCodeAt(0) & 0xff
    }
    const next = set[i + 1]
    if (next === undefined) {
      i++
      return 92
    }
    const simple: Record<string, number> = { '\\': 92, a: 7, b: 8, f: 12, n: 10, r: 13, t: 9, v: 11 }
    if (simple[next] !== undefined) {
      i += 2
      return simple[next]!
    }
    const octal = /^[0-7]{1,3}/.exec(set.slice(i + 1))
    if (octal) {
      i += 1 + octal[0].length
      return parseInt(octal[0], 8) & 0xff
    }
    i += 2
    return next.charCodeAt(0) & 0xff
  }
  while (i < set.length) {
    if (set[i] === '[' && set[i + 1] === ':') {
      const end = set.indexOf(':]', i + 2)
      if (end !== -1) {
        const name = set.slice(i + 2, end)
        const cls = CLASSES[name]
        if (cls === undefined) return `invalid character class`
        for (let c = 0; c < 256; c++) if (cls(c)) out.push(c)
        i = end + 2
        continue
      }
    }
    const start = i
    const a = readChar()
    if (set[i] === '-' && i + 1 < set.length) {
      i++
      const b = readChar()
      if (b < a) return `range-endpoints of '${set.slice(start, i)}' are in reverse collating sequence order`
      for (let c = a; c <= b; c++) out.push(c)
      continue
    }
    out.push(a)
  }
  return out
}

export const tr: Builtin = async (ctx) => {
  const flags = parseFlags('tr', ctx.argv, SPECS.tr, ctx.line)
  const deleting = has(flags, 'd')
  const operands = flags.operands
  if (operands.length === 0 || (!deleting && operands.length === 1)) {
    await ctx.stderr(`tr: missing operand${operands.length === 1 ? ` after '${operands[0]}'\nTwo strings must be given when translating.` : ''}\n${tryHelp('tr')}`)
    return 1
  }
  if ((deleting && operands.length > 1) || operands.length > 2) {
    await ctx.stderr(`tr: extra operand '${operands[deleting ? 1 : 2]}'\n${deleting ? 'Only one string may be given when deleting without squeezing repeats.\n' : ''}${tryHelp('tr')}`)
    return 1
  }
  const set1 = parseSet(utf8AsLatin1(operands[0]!))
  if (typeof set1 === 'string') {
    await ctx.stderr(`tr: ${set1}\n`)
    return 1
  }
  const stdin = guardedStdin(ctx, (detail) => ctx.stderr(`tr: read error: ${detail}\n`))
  const input = await collectBytes(stdin.stream)
  if (stdin.failed()) return 1
  const out = new Uint8Array(input.length)
  let n = 0
  if (deleting) {
    const drop = new Set(set1)
    for (const b of input) if (!drop.has(b)) out[n++] = b
    await ctx.stdout(out.subarray(0, n))
    return 0
  }
  const set2 = parseSet(utf8AsLatin1(operands[1]!))
  if (typeof set2 === 'string') {
    await ctx.stderr(`tr: ${set2}\n`)
    return 1
  }
  if (set2.length === 0) {
    await ctx.stderr(`tr: when not truncating set1, string2 must be non-empty\n`)
    return 1
  }
  const map = new Map<number, number>()
  for (let i = 0; i < set1.length; i++) {
    const target = set2[Math.min(i, set2.length - 1)]!
    // A byte listed twice takes its last mapping, as GNU tr does.
    map.set(set1[i]!, target)
  }
  for (const b of input) out[n++] = map.get(b) ?? b
  await ctx.stdout(out.subarray(0, n))
  return 0
}

