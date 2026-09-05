/**
 * The statement list tinybash executes. Everything here is exactly what the
 * grammar in `docs/demi-next/tinybash.md` admits; anything else never becomes
 * an AST, it becomes an `outside` result.
 */

/** A piece of a word before expansion. Quoted text never globs or splits. */
export type WordPart =
  | { kind: 'text'; text: string; quoted: boolean }
  | { kind: 'param'; name: string; quoted: boolean }
  /** A leading unquoted `~` (alone or before `/`), expanded to the home. */
  | { kind: 'tilde' }

export interface Word {
  parts: WordPart[]
  line: number
}

export interface Assignment {
  name: string
  value: Word
  line: number
}

export type Redirect =
  | { kind: 'file'; target: 'stdout' | 'stderr' | 'both'; mode: 'truncate' | 'append'; path: Word; line: number }
  | { kind: 'input'; path: Word; line: number }
  | { kind: 'stderr-to-stdout'; line: number }
  | { kind: 'heredoc'; body: HeredocBody; line: number }
  | { kind: 'herestring'; word: Word; line: number }

/** A heredoc body: literal when the delimiter was quoted, otherwise `$NAME` expands. */
export type HeredocBody = { kind: 'literal'; text: string } | { kind: 'expand'; parts: WordPart[] }

export interface Command {
  assignments: Assignment[]
  words: Word[]
  redirects: Redirect[]
  line: number
}

export interface Pipeline {
  commands: Command[]
  line: number
}

export interface List {
  first: Pipeline
  rest: { op: '&&' | '||'; pipeline: Pipeline }[]
}

export interface Script {
  statements: List[]
}
