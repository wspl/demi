import type { Builtin } from './io'
import type { FlagSpec } from './flags'
import { parseFlags } from './flags'
import { outside } from '../outside/reasons'
import { cat, catSpec } from './cat'
import { cd } from './cd'
import { cp, cpSpec, mv, mvSpec } from './cp-mv'
import { cut, cutSpec } from './cut'
import { echo } from './echo'
import { find, findPaths } from './find'
import { grep, grepPaths, grepSpec } from './grep'
import { head, headSpec, headTailPaths, tail, tailSpec } from './head-tail'
import { ls, lsSpec } from './ls'
import { mkdir, mkdirSpec } from './mkdir'
import { printf, printfPaths } from './printf'
import { falseBuiltin, pwd, pwdSpec, trueBuiltin } from './pwd-true-false'
import { rm, rmSpec } from './rm'
import { sed, sedPaths, sedSpec } from './sed'
import { sort, sortSpec } from './sort'
import { makeTest, testPaths } from './test'
import { touch, touchSpec } from './touch'
import { tr, trSpec } from './tr'
import { uniq, uniqSpec } from './uniq'
import { wc, wcSpec } from './wc'

/**
 * A builtin's entry in the whitelist: its flags, how to find the path operands
 * at parse time, and the implementation. `paths` throws `OutsideError` for a
 * flag or form outside the whitelist; it is the parse-first check.
 */
export interface BuiltinEntry {
  flags: FlagSpec | null
  paths: (argv: readonly string[], line: number) => string[]
  run: Builtin
}

const fileOperands = (name: string, spec: FlagSpec): BuiltinEntry['paths'] => (argv, line) =>
  parseFlags(name, argv, spec, line).operands.filter((operand) => operand !== '-')

function entry(name: string, spec: FlagSpec, run: Builtin, paths?: BuiltinEntry['paths']): [string, BuiltinEntry] {
  return [name, { flags: spec, paths: paths ?? fileOperands(name, spec), run }]
}

const none: BuiltinEntry['paths'] = () => []

export const BUILTINS: ReadonlyMap<string, BuiltinEntry> = new Map<string, BuiltinEntry>([
  entry('grep', grepSpec, grep, grepPaths),
  entry('head', headSpec, head, headTailPaths('head')),
  entry('tail', tailSpec, tail, headTailPaths('tail')),
  entry('cat', catSpec, cat),
  ['echo', { flags: null, paths: none, run: echo }],
  ['printf', { flags: null, paths: printfPaths, run: printf }],
  entry('ls', lsSpec, ls),
  ['find', { flags: null, paths: findPaths, run: find }],
  entry('wc', wcSpec, wc),
  entry('sort', sortSpec, sort),
  entry('uniq', uniqSpec, uniq),
  entry('cut', cutSpec, cut),
  entry('tr', trSpec, tr, (argv, line) => {
    parseFlags('tr', argv, trSpec, line)
    return []
  }),
  entry('sed', sedSpec, sed, sedPaths),
  entry('mkdir', mkdirSpec, mkdir),
  entry('rm', rmSpec, rm),
  entry('mv', mvSpec, mv),
  entry('cp', cpSpec, cp),
  entry('touch', touchSpec, touch),
  entry('pwd', pwdSpec, pwd),
  ['true', { flags: null, paths: none, run: trueBuiltin }],
  ['false', { flags: null, paths: none, run: falseBuiltin }],
  ['test', { flags: null, paths: (argv, line) => testPaths('test', argv, line), run: makeTest('test') }],
  ['[', { flags: null, paths: (argv, line) => testPaths('[', argv, line), run: makeTest('[') }],
  ['cd', { flags: null, paths: (argv, line) => cdPaths(argv, line), run: cd }],
])

/** `cd` takes one path or none; `cd -` and every option are outside (bash reports too many arguments itself). */
function cdPaths(argv: readonly string[], line: number): string[] {
  const first = argv[0]
  if (first !== undefined && first.startsWith('-')) outside({ kind: 'flag', program: 'cd', flag: first, line })
  return argv.length === 1 ? [first!] : []
}
