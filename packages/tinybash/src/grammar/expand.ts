import type { HeredocBody, Word, WordPart } from './ast'
import { expandGlob, hasGlobChars } from './glob'
import type { HostFileSystem } from '@demicodes/shell'

export interface ExpansionScope {
  home: string
  cwd: string
  /** `NAME` → value; `PWD` is always the cwd, as in bash. */
  vars: Readonly<Record<string, string>>
}

function lookup(scope: ExpansionScope, name: string): string {
  if (name === 'PWD') return scope.cwd
  return scope.vars[name] ?? ''
}

/**
 * A character of an expanded word, tagged with whether glob characters in it
 * are literal (quoted or produced by a parameter that was quoted).
 */
export interface Piece {
  text: string
  literal: boolean
}

/**
 * Tilde and parameter expansion into fields, with bash's word splitting on
 * unquoted parameter results (default IFS). Each field is a list of pieces;
 * globbing decides per piece whether `*?[` are pattern characters.
 */
export function expandToFields(word: Word, scope: ExpansionScope): Piece[][] {
  const fields: Piece[][] = []
  let current: Piece[] | null = null
  let hasQuotedEmpty = false
  const push = (piece: Piece) => {
    if (current === null) current = []
    if (piece.text.length > 0 || piece.literal) current.push(piece)
  }
  for (const part of word.parts) {
    if (part.kind === 'tilde') {
      push({ text: scope.home, literal: true })
    } else if (part.kind === 'text') {
      if (part.quoted && part.text.length === 0) hasQuotedEmpty = true
      push({ text: part.text, literal: part.quoted })
    } else if (part.quoted) {
      push({ text: lookup(scope, part.name), literal: true })
    } else {
      // Unquoted `$NAME`: split on blanks; each inner field ends the current one.
      const value = lookup(scope, part.name)
      const segments = value.split(/[ \t\n]+/)
      const leadingBlank = /^[ \t\n]/.test(value)
      const trailingBlank = /[ \t\n]$/.test(value)
      for (let i = 0; i < segments.length; i++) {
        const segment = segments[i]!
        const startsField = i > 0 || leadingBlank
        if (startsField) {
          if (current !== null) fields.push(current)
          current = null
        }
        if (segment.length > 0) push({ text: segment, literal: false })
      }
      if (trailingBlank && current !== null) {
        fields.push(current)
        current = null
      }
    }
  }
  if (current !== null) fields.push(current)
  // A word that was only an empty quoted string is still one empty argument; a bare
  // unquoted `$EMPTY` disappears entirely.
  if (fields.length === 0 && hasQuotedEmpty) return [[{ text: '', literal: true }]]
  return fields.filter((field) => field.length > 0 || hasQuotedEmpty)
}

export function fieldText(field: readonly Piece[]): string {
  return field.map((piece) => piece.text).join('')
}

/** Expansion without splitting or globbing: assignment values, redirect targets, here-strings. */
export function expandSingle(word: Word, scope: ExpansionScope): string {
  let out = ''
  for (const part of word.parts) {
    if (part.kind === 'tilde') out += scope.home
    else if (part.kind === 'text') out += part.text
    else out += lookup(scope, part.name)
  }
  return out
}

/** Full expansion of a command's words into argv: tilde, parameter, splitting, globbing. */
export async function expandArgv(words: readonly Word[], scope: ExpansionScope, fs: HostFileSystem): Promise<string[]> {
  const argv: string[] = []
  for (const word of words) {
    for (const field of expandToFields(word, scope)) {
      if (hasGlobChars(field)) argv.push(...(await expandGlob(field, scope.cwd, fs)))
      else argv.push(fieldText(field))
    }
  }
  return argv
}

export function expandHeredoc(body: HeredocBody, scope: ExpansionScope): string {
  if (body.kind === 'literal') return body.text
  return expandSingle({ parts: body.parts, line: 0 }, scope)
}

/** A word roughly as the script wrote it (`$NAME` and `~` unexpanded), for bash's own messages about it. */
export function wordSource(word: Word): string {
  return word.parts.map((part) => (part.kind === 'tilde' ? '~' : part.kind === 'param' ? `$${part.name}` : part.text)).join('')
}

/** The static text of a word, with parameters resolved, for parse-time checks. */
export function staticText(parts: readonly WordPart[], scope: ExpansionScope): string {
  return expandSingle({ parts: [...parts], line: 0 }, scope)
}
