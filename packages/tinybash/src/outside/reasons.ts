/**
 * Why a script is outside the subset. The kinds match the rows of the refusal
 * table in `docs/demi-next/tinybash.md`; `message` is the one line an embedder
 * without a machine shows, with exit code 2.
 */
export type OutsideReason =
  /** A construct the grammar does not admit: substitution, control flow, job control, brace expansion. */
  | { kind: 'grammar'; found: string; why: string; wayOut: string; line: number }
  /** A command word that is neither a builtin nor a root. */
  | { kind: 'program'; name: string; line: number }
  /** A builtin flag or form outside its whitelist. */
  | { kind: 'flag'; program: string; flag: string; line: number }
  /** An absolute path outside the namespace. */
  | { kind: 'path'; path: string; line: number }
  /** A grep pattern with no faithful translation. */
  | { kind: 'pattern'; pattern: string; line: number }
  /** Not a script: unterminated quote or heredoc, a carriage return. */
  | { kind: 'syntax'; found: string; line: number }

export class OutsideError extends Error {
  constructor(readonly reason: OutsideReason) {
    super(refusalMessage(reason))
    this.name = 'OutsideError'
  }
}

export function outside(reason: OutsideReason): never {
  throw new OutsideError(reason)
}

/** Programs refused outright, with the way out named. */
const REFUSED_PROGRAMS: Record<string, string> = {
  awk: 'a machine',
  jq: 'a machine',
  xargs: 'a machine',
  perl: 'a machine',
  python: 'a machine',
  python3: 'a machine',
  export: 'a plain NAME=value assignment',
  source: 'a machine',
  eval: 'a machine',
  exit: 'ending the script',
  sleep: 'a machine',
  kill: 'a machine',
}

export function refusalMessage(reason: OutsideReason): string {
  const at = `tinybash: line ${reason.line}:`
  switch (reason.kind) {
    case 'grammar':
      return `${at} ${reason.found}: ${reason.why}; ${reason.wayOut}`
    case 'program': {
      const wayOut = REFUSED_PROGRAMS[reason.name] ?? 'a machine'
      return `${at} ${reason.name}: no such program here; ${wayOut}`
    }
    case 'flag':
      if (reason.program === 'sed' && (reason.flag === '-i' || reason.flag.startsWith('s'))) {
        return `${at} sed ${reason.flag}: not implemented faithfully; \`demi file edit\` for edits, or a machine`
      }
      return `${at} ${reason.program} ${reason.flag}: not implemented faithfully; the listed flags, or a machine`
    case 'path':
      return `${at} ${reason.path}: no such place here; a path under the home, or a machine`
    case 'pattern':
      return `${at} grep pattern ${JSON.stringify(reason.pattern)}: dialect; -F, or a simpler pattern`
    case 'syntax':
      return `${at} ${reason.found}: not a script; fix the script`
  }
}
