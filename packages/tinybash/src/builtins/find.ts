import type { Builtin, BuiltinContext } from './io'
import { outside } from '../outside/reasons'
import { quoteC, strerror } from './errors'
import { latin1Bytes } from '../exec/stream'
import { bracketToRegex, escapeRegex } from '../grammar/glob'

interface Expression {
  maxDepth: number | null
  name: { pattern: string; ignoreCase: boolean } | null
  type: 'f' | 'd' | null
}

/** Splits argv into start paths and the accepted predicates; throws `OutsideError` beyond the table. */
function parseFind(argv: readonly string[], line: number): { paths: string[]; expression: Expression; error: string | null } {
  const paths: string[] = []
  let i = 0
  while (i < argv.length && !argv[i]!.startsWith('-')) paths.push(argv[i++]!)
  const expression: Expression = { maxDepth: null, name: null, type: null }
  let sawTest = false
  while (i < argv.length) {
    const arg = argv[i]!
    const needValue = (): string => {
      if (i + 1 >= argv.length) throw new FindError(`missing argument to \`${arg}'`)
      return argv[(i += 2) - 1]!
    }
    if (arg === '-maxdepth') {
      if (sawTest) outside({ kind: 'flag', program: 'find', flag: '-maxdepth after a test', line })
      const raw = needValue()
      if (!/^\d+$/.test(raw)) throw new FindError(`Expected a positive decimal integer argument to -maxdepth, but got ${quoteC(raw).slice(1, -1) === raw ? `\`${raw}'` : raw}`)
      expression.maxDepth = Number(raw)
      continue
    }
    if (arg === '-name' || arg === '-iname') {
      expression.name = { pattern: needValue(), ignoreCase: arg === '-iname' }
      sawTest = true
      continue
    }
    if (arg === '-type') {
      const raw = needValue()
      if (raw !== 'f' && raw !== 'd') {
        if (/^[bcplsD]$/.test(raw)) outside({ kind: 'flag', program: 'find', flag: `-type ${raw}`, line })
        throw new FindError(`Unknown argument to -type: ${raw}`)
      }
      expression.type = raw
      sawTest = true
      continue
    }
    outside({ kind: 'flag', program: 'find', flag: arg, line })
  }
  return { paths: paths.length === 0 ? ['.'] : paths, expression, error: null }
}

class FindError extends Error {}

export function findPaths(argv: readonly string[], line: number): string[] {
  try {
    return parseFind(argv, line).paths
  } catch (error) {
    if (error instanceof FindError) return []
    throw error
  }
}

/** fnmatch without FNM_PERIOD: `*` and `?` match a leading dot too. */
function nameMatcher(pattern: string, ignoreCase: boolean): RegExp {
  let out = '^'
  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i]!
    if (ch === '*') out += '.*'
    else if (ch === '?') out += '.'
    else if (ch === '[') {
      const close = pattern.indexOf(']', i + 2)
      if (close === -1) out += '\\['
      else {
        out += bracketToRegex(pattern.slice(i + 1, close))
        i = close
      }
    } else if (ch === '\\' && i + 1 < pattern.length) {
      out += escapeRegex(pattern[++i]!)
    } else out += escapeRegex(ch)
  }
  return new RegExp(`${out}$`, ignoreCase ? 'i' : '')
}

export const find: Builtin = async (ctx) => {
  let parsed
  try {
    parsed = parseFind(ctx.argv, ctx.line)
  } catch (error) {
    if (!(error instanceof FindError)) throw error
    await ctx.stderr(`find: ${error.message}\n`)
    return 1
  }
  const matcher = parsed.expression.name ? nameMatcher(parsed.expression.name.pattern, parsed.expression.name.ignoreCase) : null
  let status = 0
  for (const start of parsed.paths) {
    const ok = await walk(ctx, start, 0, parsed.expression, matcher)
    if (!ok) status = 1
  }
  return status
}

async function walk(ctx: BuiltinContext, path: string, depth: number, expression: Expression, matcher: RegExp | null): Promise<boolean> {
  let stat
  try {
    stat = await ctx.fs.lstat(path, { cwd: ctx.cwd })
  } catch (error) {
    await ctx.stderr(`find: ${quoteC(path)}: ${strerror(error)}\n`)
    return false
  }
  const base = depth === 0 ? path.replace(/\/+$/, '') || '/' : path.slice(path.lastIndexOf('/') + 1)
  const baseName = depth === 0 ? base.slice(base.lastIndexOf('/') + 1) || base : base
  const typeOk = expression.type === null || (expression.type === 'f' ? stat.isFile && !stat.isSymbolicLink : stat.isDirectory && !stat.isSymbolicLink)
  const nameOk = matcher === null || matcher.test(baseName)
  if (typeOk && nameOk) await ctx.stdout(latin1Bytes(`${path}\n`))
  if (!stat.isDirectory || stat.isSymbolicLink) return true
  if (expression.maxDepth !== null && depth >= expression.maxDepth) return true
  let names: string[]
  try {
    names = await ctx.fs.readdir(path, { cwd: ctx.cwd })
  } catch (error) {
    await ctx.stderr(`find: ${quoteC(path)}: ${strerror(error)}\n`)
    return false
  }
  names.sort((x, y) => (x < y ? -1 : x > y ? 1 : 0))
  let ok = true
  for (const name of names) {
    const child = path.endsWith('/') ? `${path}${name}` : `${path}/${name}`
    if (!(await walk(ctx, child, depth + 1, expression, matcher))) ok = false
  }
  return ok
}
