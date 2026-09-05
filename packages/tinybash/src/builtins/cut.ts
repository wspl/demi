import type { Builtin } from './io'
import { type FlagSpec, parseFlags, value } from './flags'
import { lazyInputs } from './inputs'
import { encodeLatin1, utf8AsLatin1 } from '@demicodes/utils'
import { lines } from '../exec/stream'
import { tryHelp } from './errors'

export const cutSpec: FlagSpec = { switches: [], valued: ['d', 'f'] }

/** A field list like `1,3-5,7-` into a membership test (1-based). */
function parseList(list: string): ((field: number) => boolean) | null {
  const ranges: { from: number; to: number }[] = []
  for (const item of list.split(',')) {
    const match = /^(\d*)(-?)(\d*)$/.exec(item)
    if (!match || item === '' || item === '-') return null
    const [, a, dash, b] = match
    if (dash === '') {
      const n = Number(a)
      if (n === 0) return null
      ranges.push({ from: n, to: n })
    } else {
      const from = a === '' ? 1 : Number(a)
      const to = b === '' ? Number.POSITIVE_INFINITY : Number(b)
      if (from === 0 || to === 0 || from > to) return null
      ranges.push({ from, to })
    }
  }
  return (field) => ranges.some((range) => field >= range.from && field <= range.to)
}

export const cut: Builtin = async (ctx) => {
  const flags = parseFlags('cut', ctx.argv, cutSpec, ctx.line)
  const list = value(flags, 'f')
  const raw = value(flags, 'd')
  const delimiter = raw === undefined ? undefined : utf8AsLatin1(raw)
  if (list === undefined) {
    await ctx.stderr(`cut: you must specify a list of bytes, characters, or fields\n${tryHelp('cut')}`)
    return 1
  }
  if (delimiter !== undefined && delimiter.length > 1) {
    await ctx.stderr(`cut: the delimiter must be a single character\n${tryHelp('cut')}`)
    return 1
  }
  const selected = parseList(list)
  if (selected === null) {
    const bad = /^\d*-?\d*$/.test(list.split(',').find((item) => !/^\d+(-\d*)?$|^-\d+$/.test(item)) ?? '') ? 'fields are numbered from 1' : `invalid field value '${list}'`
    await ctx.stderr(`cut: ${bad}\n${tryHelp('cut')}`)
    return 1
  }
  const sep = delimiter === undefined || delimiter === '' ? '\t' : delimiter
  let status = 0
  for (const input of lazyInputs(ctx, 'cut', flags.operands)) {
    const stream = await input.open()
    if (stream === null) {
      status = 1
      continue
    }
    for await (const line of lines(stream)) {
      const fields = line.text.split(sep)
      const out = fields.length === 1 ? line.text : fields.filter((_, index) => selected(index + 1)).join(sep)
      await ctx.stdout(encodeLatin1(`${out}${line.newline ? '\n' : ''}`))
    }
    if (input.readFailed()) status = 1
  }
  return status
}
