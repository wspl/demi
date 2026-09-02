import type { Builtin } from './io'
import type { FlagSpec } from './flags'
import { parseFlags } from './flags'
import { outside } from '../outside/reasons'
import { cat } from './cat'
import { cd } from './cd'
import { cp, mv } from './cp-mv'
import { cut } from './cut'
import { echo } from './echo'
import { find, findPaths } from './find'
import { grep, grepPaths } from './grep'
import { head, headTailPaths, tail } from './head-tail'
import { ls } from './ls'
import { mkdir } from './mkdir'
import { printf, printfPaths } from './printf'
import { falseBuiltin, pwd, trueBuiltin } from './pwd-true-false'
import { rm } from './rm'
import { sed, sedPaths } from './sed'
import { sort } from './sort'
import { makeTest, testPaths } from './test'
import { touch } from './touch'
import { tr } from './tr'
import { uniq } from './uniq'
import { wc } from './wc'

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

export const SPECS = {
  grep: { switches: ['n', 'i', 'v', 'c', 'l', 'r', 'E', 'F'], valued: ['A', 'B', 'C'] },
  head: { switches: [], valued: ['n', 'c'] },
  tail: { switches: [], valued: ['n', 'c'] },
  cat: { switches: ['n'], valued: [] },
  ls: { switches: ['l', 'a', '1', 'R'], valued: [] },
  wc: { switches: ['l', 'w', 'c'], valued: [] },
  sort: { switches: ['r', 'n', 'u'], valued: ['k'] },
  uniq: { switches: ['c'], valued: [] },
  cut: { switches: [], valued: ['d', 'f'] },
  tr: { switches: ['d'], valued: [] },
  sed: { switches: ['n'], valued: [] },
  mkdir: { switches: ['p'], valued: [] },
  rm: { switches: ['r', 'f'], valued: [] },
  cp: { switches: ['r'], valued: [] },
  mv: { switches: [], valued: [] },
  touch: { switches: [], valued: [] },
  pwd: { switches: [], valued: [] },
} satisfies Record<string, FlagSpec>

export const BUILTINS: ReadonlyMap<string, BuiltinEntry> = new Map<string, BuiltinEntry>([
  entry('grep', SPECS.grep, grep, grepPaths),
  entry('head', SPECS.head, head, headTailPaths('head')),
  entry('tail', SPECS.tail, tail, headTailPaths('tail')),
  entry('cat', SPECS.cat, cat),
  ['echo', { flags: null, paths: none, run: echo }],
  ['printf', { flags: null, paths: printfPaths, run: printf }],
  entry('ls', SPECS.ls, ls),
  ['find', { flags: null, paths: findPaths, run: find }],
  entry('wc', SPECS.wc, wc),
  entry('sort', SPECS.sort, sort),
  entry('uniq', SPECS.uniq, uniq),
  entry('cut', SPECS.cut, cut),
  entry('tr', SPECS.tr, tr, (argv, line) => {
    parseFlags('tr', argv, SPECS.tr, line)
    return []
  }),
  entry('sed', SPECS.sed, sed, sedPaths),
  entry('mkdir', SPECS.mkdir, mkdir),
  entry('rm', SPECS.rm, rm),
  entry('mv', SPECS.mv, mv),
  entry('cp', SPECS.cp, cp),
  entry('touch', SPECS.touch, touch),
  entry('pwd', SPECS.pwd, pwd),
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
