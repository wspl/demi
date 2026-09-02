import type { Builtin, BuiltinContext } from './io'
import { parseFlags, has, value } from './flags'
import { SPECS } from './table'
import { translatePattern } from './grep-pattern'
import { lines } from '../exec/stream'
import { strerror } from './errors'
import { collectBytes, compareUtf8Bytes, encodeLatin1, errorCode, utf8AsLatin1 } from '@demicodes/utils'
import { guardedStdin } from './inputs'

/** The files after the pattern; also the parse-time check of the flags and the pattern. */
export function grepPaths(argv: readonly string[], line: number): string[] {
  const flags = parseFlags('grep', argv, SPECS.grep, line)
  if (flags.operands.length === 0) return []
  translatePattern(flags.operands[0]!, has(flags, 'F') ? 'fixed' : has(flags, 'E') ? 'extended' : 'basic', has(flags, 'i'), line)
  return flags.operands.slice(1).filter((operand) => operand !== '-')
}

interface Options {
  regex: RegExp
  invert: boolean
  count: boolean
  listFiles: boolean
  numbers: boolean
  before: number
  after: number
  withNames: boolean
}

function contextLength(raw: string | undefined): number | null {
  if (raw === undefined) return 0
  if (!/^\d+$/.test(raw)) return null
  return Number(raw)
}

export const grep: Builtin = async (ctx) => {
  const flags = parseFlags('grep', ctx.argv, SPECS.grep, ctx.line)
  if (flags.operands.length === 0) {
    await ctx.stderr(`Usage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information.\n`)
    return 2
  }
  const c = contextLength(value(flags, 'C'))
  const a = contextLength(value(flags, 'A'))
  const b = contextLength(value(flags, 'B'))
  for (const [raw, parsed] of [[value(flags, 'C'), c], [value(flags, 'A'), a], [value(flags, 'B'), b]] as const) {
    if (parsed === null) {
      await ctx.stderr(`grep: ${raw}: invalid context length argument\n`)
      return 2
    }
  }
  const recursive = has(flags, 'r')
  const operands = flags.operands.slice(1)
  const options: Options = {
    regex: translatePattern(utf8AsLatin1(flags.operands[0]!), has(flags, 'F') ? 'fixed' : has(flags, 'E') ? 'extended' : 'basic', has(flags, 'i'), ctx.line),
    invert: has(flags, 'v'),
    count: has(flags, 'c'),
    listFiles: has(flags, 'l'),
    numbers: has(flags, 'n'),
    before: value(flags, 'B') !== undefined ? b! : c!,
    after: value(flags, 'A') !== undefined ? a! : c!,
    withNames: recursive || operands.length > 1,
  }
  let matched = false
  let errored = false
  const files = operands.length === 0 ? (recursive ? [{ path: '.', display: '' }] : [{ path: '-', display: '(standard input)' }]) : operands.map((path) => ({ path, display: path }))
  for (const file of files) {
    const result = await grepPath(ctx, file.path, file.display, options, recursive, true)
    if (result === 'error') errored = true
    else if (result) matched = true
  }
  if (errored) return 2
  return matched ? 0 : 1
}

async function grepPath(ctx: BuiltinContext, path: string, display: string, options: Options, recursive: boolean, topLevel: boolean): Promise<boolean | 'error'> {
  if (path === '-') {
    const stdin = guardedStdin(ctx, (detail) => ctx.stderr(`grep: ${display}: ${detail}\n`))
    const matched = await grepBytes(ctx, await collectBytes(stdin.stream), display, options)
    return stdin.failed() ? 'error' : matched
  }
  let stat
  try {
    stat = topLevel ? await ctx.fs.stat(path, { cwd: ctx.cwd }) : await ctx.fs.lstat(path, { cwd: ctx.cwd })
  } catch (error) {
    await ctx.stderr(`grep: ${display}: ${strerror(error)}\n`)
    return 'error'
  }
  if (stat.isSymbolicLink && !topLevel) return false
  if (stat.isDirectory) {
    if (!recursive) {
      await ctx.stderr(`grep: ${display}: Is a directory\n`)
      return 'error'
    }
    let names: string[]
    try {
      names = await ctx.fs.readdir(path, { cwd: ctx.cwd })
    } catch (error) {
      await ctx.stderr(`grep: ${display}: ${strerror(error)}\n`)
      return 'error'
    }
    names.sort(compareUtf8Bytes)
    let matched = false
    let errored = false
    for (const name of names) {
      const childPath = path.endsWith('/') ? `${path}${name}` : `${path}/${name}`
      const childDisplay = display === '' ? name : display.endsWith('/') ? `${display}${name}` : `${display}/${name}`
      const result = await grepPath(ctx, childPath, childDisplay, options, recursive, false)
      if (result === 'error') errored = true
      else if (result) matched = true
    }
    return errored ? 'error' : matched
  }
  try {
    return await grepBytes(ctx, await ctx.fs.readFile(path, { cwd: ctx.cwd }), display, options)
  } catch (error) {
    if (errorCode(error) === 'EISDIR') return false
    await ctx.stderr(`grep: ${display}: ${strerror(error)}\n`)
    return 'error'
  }
}


async function* oneChunk(bytes: Uint8Array): AsyncIterable<Uint8Array> {
  if (bytes.length > 0) yield bytes
}

async function grepBytes(ctx: BuiltinContext, bytes: Uint8Array, display: string, options: Options): Promise<boolean> {
  const binary = bytes.includes(0)
  const all: string[] = []
  for await (const line of lines(oneChunk(bytes))) all.push(line.text)
  const selected = all.map((text) => options.regex.test(text) !== options.invert)
  const count = selected.filter(Boolean).length
  const prefix = options.withNames ? utf8AsLatin1(display) : ''
  if (options.count) {
    await ctx.stdout(encodeLatin1(`${prefix ? `${prefix}:` : ''}${count}\n`))
    return count > 0
  }
  if (options.listFiles) {
    if (count > 0) await ctx.stdout(encodeLatin1(`${utf8AsLatin1(display)}\n`))
    return count > 0
  }
  if (count === 0) return false
  if (binary) {
    await ctx.stderr(`grep: ${display}: binary file matches\n`)
    return true
  }
  const context = options.before > 0 || options.after > 0
  let lastPrinted = -1
  for (let i = 0; i < all.length; i++) {
    if (!selected[i]) continue
    const from = Math.max(0, i - options.before, lastPrinted + 1)
    if (context && lastPrinted >= 0 && from > lastPrinted + 1) await ctx.stdout('--\n')
    for (let j = from; j < i; j++) {
      await ctx.stdout(encodeLatin1(`${prefix ? `${prefix}-` : ''}${options.numbers ? `${j + 1}-` : ''}${all[j]}\n`))
    }
    await ctx.stdout(encodeLatin1(`${prefix ? `${prefix}:` : ''}${options.numbers ? `${i + 1}:` : ''}${all[i]}\n`))
    lastPrinted = i
    for (let j = i + 1; j <= Math.min(all.length - 1, i + options.after); j++) {
      if (selected[j]) break
      await ctx.stdout(encodeLatin1(`${prefix ? `${prefix}-` : ''}${options.numbers ? `${j + 1}-` : ''}${all[j]}\n`))
      lastPrinted = j
    }
  }
  return true
}
