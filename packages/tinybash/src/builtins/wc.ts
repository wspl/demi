import type { Builtin } from './io'
import { parseFlags, has } from './flags'
import { SPECS } from './table'
import { collect } from '../exec/stream'
import { strerror } from './errors'

interface Counts {
  lines: number
  words: number
  bytes: number
}

function count(bytes: Uint8Array): Counts {
  let lines = 0
  let words = 0
  let inWord = false
  for (const b of bytes) {
    if (b === 0x0a) lines++
    // C-locale isspace: space, \t \n \v \f \r
    const space = b === 0x20 || (b >= 0x09 && b <= 0x0d)
    if (space) inWord = false
    else if (!inWord) {
      inWord = true
      words++
    }
  }
  return { lines, words, bytes: bytes.length }
}

export const wc: Builtin = async (ctx) => {
  const flags = parseFlags('wc', ctx.argv, SPECS.wc, ctx.line)
  const show = {
    lines: has(flags, 'l'),
    words: has(flags, 'w'),
    bytes: has(flags, 'c'),
  }
  if (!show.lines && !show.words && !show.bytes) show.lines = show.words = show.bytes = true
  const selected = [show.lines, show.words, show.bytes].filter(Boolean).length
  const operands = flags.operands.length === 0 ? ['-'] : flags.operands
  // GNU: the column width comes from the total size of the regular-file operands;
  // stdin and other non-regular inputs force at least 7. A single count for a
  // single operand is printed without padding.
  let regularTotal = 0
  let minimumWidth = 1
  let status = 0
  const results: { name: string | null; counts: Counts }[] = []
  for (const name of operands) {
    if (name === '-') {
      minimumWidth = 7
      results.push({ name: null, counts: count(await collect(ctx.stdin)) })
      continue
    }
    try {
      const stat = await ctx.fs.stat(name, { cwd: ctx.cwd })
      if (stat.isFile) regularTotal += stat.size
      else minimumWidth = 7
      if (stat.isDirectory) {
        // GNU reads the directory, fails with EISDIR, and still reports its zero counts.
        await ctx.stderr(`wc: ${name}: Is a directory\n`)
        results.push({ name, counts: { lines: 0, words: 0, bytes: 0 } })
        status = 1
        continue
      }
      const bytes = await ctx.fs.readFile(name, { cwd: ctx.cwd })
      results.push({ name, counts: count(bytes) })
    } catch (error) {
      await ctx.stderr(`wc: ${name}: ${strerror(error)}\n`)
      status = 1
    }
  }
  let width = 1
  for (let total = regularTotal; total >= 10; total = Math.floor(total / 10)) width++
  if (width < minimumWidth) width = minimumWidth
  if (selected === 1 && operands.length === 1) width = 1
  const format = (counts: Counts, name: string | null) => {
    const columns: string[] = []
    if (show.lines) columns.push(String(counts.lines).padStart(width))
    if (show.words) columns.push(String(counts.words).padStart(width))
    if (show.bytes) columns.push(String(counts.bytes).padStart(width))
    return `${columns.join(' ')}${name === null ? '' : ` ${name}`}\n`
  }
  for (const result of results) await ctx.stdout(format(result.counts, result.name))
  if (operands.length > 1) {
    const total = results.reduce((acc, r) => ({ lines: acc.lines + r.counts.lines, words: acc.words + r.counts.words, bytes: acc.bytes + r.counts.bytes }), { lines: 0, words: 0, bytes: 0 })
    await ctx.stdout(format(total, 'total'))
  }
  return status
}
