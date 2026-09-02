import { outside } from '../outside/reasons'

/**
 * The declaration of a builtin's whitelist: which single-letter flags it takes,
 * which of those carry a value, and how its operands are read. Everything the
 * table does not list is outside the subset.
 */
export interface FlagSpec {
  /** Flags without a value, e.g. `n` for `-n`. */
  switches: readonly string[]
  /** Flags that take a value, attached (`-n5`) or separate (`-n 5`). */
  valued: readonly string[]
  /** Whether `--` and operands may be interleaved with flags (GNU permutes; `echo`/`test` do not parse flags at all). */
  permute?: boolean
}

export interface ParsedFlags {
  /** Switch flags seen, in order (repeats kept). */
  switches: string[]
  /** Valued flags with their values, in order. */
  values: { flag: string; value: string }[]
  operands: string[]
}

export function parseFlags(program: string, argv: readonly string[], spec: FlagSpec, line: number): ParsedFlags {
  const result: ParsedFlags = { switches: [], values: [], operands: [] }
  const permute = spec.permute ?? true
  let i = 0
  while (i < argv.length) {
    const arg = argv[i]!
    if (arg === '--') {
      result.operands.push(...argv.slice(i + 1))
      break
    }
    if (arg === '-' || !arg.startsWith('-')) {
      result.operands.push(arg)
      i++
      if (!permute) {
        result.operands.push(...argv.slice(i))
        break
      }
      continue
    }
    if (arg.startsWith('--')) outside({ kind: 'flag', program, flag: arg, line })
    let j = 1
    while (j < arg.length) {
      const letter = arg[j]!
      if (spec.valued.includes(letter)) {
        const attached = arg.slice(j + 1)
        if (attached.length > 0) {
          result.values.push({ flag: letter, value: attached })
        } else {
          i++
          if (i >= argv.length) outside({ kind: 'flag', program, flag: `-${letter} without a value`, line })
          result.values.push({ flag: letter, value: argv[i]! })
        }
        j = arg.length
        break
      }
      if (!spec.switches.includes(letter)) outside({ kind: 'flag', program, flag: `-${letter}`, line })
      result.switches.push(letter)
      j++
    }
    i++
  }
  return result
}

export function has(flags: ParsedFlags, letter: string): boolean {
  return flags.switches.includes(letter)
}

/** The last value given for a flag, or undefined. */
export function value(flags: ParsedFlags, letter: string): string | undefined {
  for (let i = flags.values.length - 1; i >= 0; i--) {
    if (flags.values[i]!.flag === letter) return flags.values[i]!.value
  }
  return undefined
}
