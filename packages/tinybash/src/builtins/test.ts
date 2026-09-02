import type { Builtin } from './io'
import { outside } from '../outside/reasons'

const PATH_UNARY = new Set(['-e', '-f', '-d', '-s'])
const STRING_UNARY = new Set(['-z', '-n'])
const BINARY = new Set(['=', '!='])

/**
 * The expression's operands after `[`'s closing bracket, or the form beyond
 * the table. At parse time that form is outside the subset; at run time — a
 * glob having expanded into extra words — it is the error bash reports.
 */
function operands(program: string, argv: readonly string[]): { args: string[] } | { beyond: string; bashError: string } {
  let args = [...argv]
  if (program === '[') {
    if (args[args.length - 1] !== ']') return { args } // a runtime error: missing `]`
    args = args.slice(0, -1)
  }
  if (args.length <= 1) return { args }
  if (args.length === 2) {
    if (PATH_UNARY.has(args[0]!) || STRING_UNARY.has(args[0]!)) return { args }
    return { beyond: args[0]!, bashError: `${args[0]}: unary operator expected` }
  }
  if (args.length === 3) {
    if (BINARY.has(args[1]!)) return { args }
    return { beyond: args.join(' '), bashError: `${args[1]}: binary operator expected` }
  }
  return { beyond: args.join(' '), bashError: 'too many arguments' }
}

export function testPaths(program: string, argv: readonly string[], line: number): string[] {
  const parsed = operands(program, argv)
  if ('beyond' in parsed) outside({ kind: 'flag', program, flag: parsed.beyond, line })
  const args = parsed.args
  if (args.length === 2 && PATH_UNARY.has(args[0]!)) return [args[1]!]
  return []
}

export function makeTest(program: 'test' | '['): Builtin {
  return async (ctx) => {
    if (program === '[' && ctx.argv[ctx.argv.length - 1] !== ']') {
      await ctx.stderr(`bash: line ${ctx.line}: [: missing \`]'\n`)
      return 2
    }
    const parsed = operands(program, ctx.argv)
    if ('beyond' in parsed) {
      await ctx.stderr(`bash: line ${ctx.line}: ${program}: ${parsed.bashError}\n`)
      return 2
    }
    const args = parsed.args
    if (args.length === 0) return 1
    if (args.length === 1) return args[0]!.length > 0 ? 0 : 1
    if (args.length === 2) {
      const [op, operand] = args as [string, string]
      if (op === '-z') return operand.length === 0 ? 0 : 1
      if (op === '-n') return operand.length > 0 ? 0 : 1
      try {
        const stat = await ctx.fs.stat(operand, { cwd: ctx.cwd })
        if (op === '-e') return 0
        if (op === '-f') return stat.isFile ? 0 : 1
        if (op === '-d') return stat.isDirectory ? 0 : 1
        return stat.size > 0 ? 0 : 1
      } catch {
        return 1
      }
    }
    const [a, op, b] = args as [string, string, string]
    return (op === '=' ? a === b : a !== b) ? 0 : 1
  }
}
