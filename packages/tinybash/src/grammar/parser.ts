import type { Assignment, Command, List, Pipeline, Redirect, Script, Word, WordPart } from './ast'
import { type Token, readName, tokenize } from '../grammar/lexer'
import { outside } from '../outside/reasons'

/** Words that start a construct bash would treat as syntax, refused when they lead a command. */
const RESERVED = new Set([
  'if', 'then', 'else', 'elif', 'fi', 'for', 'while', 'until', 'do', 'done', 'case', 'esac', 'in',
  'function', 'select', 'time', 'coproc', '[[', ']]', '!',
])

/** Parses a whole script into its statement list, or throws `OutsideError`. */
export function parseScript(script: string): Script {
  return new Parser(tokenize(script)).script()
}

class Parser {
  private index = 0

  constructor(private readonly tokens: Token[]) {}

  private peek(): Token {
    return this.tokens[this.index]!
  }

  private next(): Token {
    return this.tokens[this.index++]!
  }

  private isOp(token: Token, ...ops: string[]): boolean {
    return token.kind === 'op' && ops.includes(token.op)
  }

  script(): Script {
    const statements: List[] = []
    for (;;) {
      while (this.isOp(this.peek(), ';', 'newline')) this.next()
      if (this.peek().kind === 'eof') break
      statements.push(this.list())
      const after = this.peek()
      if (after.kind === 'eof') break
      if (!this.isOp(after, ';', 'newline')) this.unexpected(after)
    }
    return { statements }
  }

  private list(): List {
    const first = this.pipeline()
    const rest: List['rest'] = []
    while (this.isOp(this.peek(), '&&', '||')) {
      const op = (this.next() as Extract<Token, { kind: 'op' }>).op as '&&' | '||'
      // bash lets the next pipeline start on a following line after `&&` / `||`.
      while (this.isOp(this.peek(), 'newline')) this.next()
      rest.push({ op, pipeline: this.pipeline() })
    }
    return { first, rest }
  }

  private pipeline(): Pipeline {
    const first = this.command()
    const commands = [first]
    while (this.isOp(this.peek(), '|')) {
      this.next()
      while (this.isOp(this.peek(), 'newline')) this.next()
      commands.push(this.command())
    }
    return { commands, line: first.line }
  }

  private command(): Command {
    const start = this.peek()
    const line = start.kind === 'eof' ? start.line : start.kind === 'word' ? start.word.line : start.line
    const assignments: Assignment[] = []
    const words: Word[] = []
    const redirects: Redirect[] = []
    for (;;) {
      const token = this.peek()
      if (token.kind === 'word') {
        this.next()
        const assignment = words.length === 0 ? asAssignment(token.word) : null
        if (assignment) {
          assignments.push(assignment)
          continue
        }
        if (words.length === 0) this.checkReserved(token.word)
        words.push(token.word)
        continue
      }
      if (token.kind === 'heredoc') {
        this.next()
        if (token.body === null) outside({ kind: 'syntax', found: 'here-document without a body', line: token.line })
        redirects.push({ kind: 'heredoc', body: token.body, line: token.line })
        continue
      }
      if (token.kind === 'op') {
        const redirect = this.redirect(token)
        if (redirect) {
          redirects.push(redirect)
          continue
        }
      }
      break
    }
    if (words.length === 0) {
      if (assignments.length === 0) this.unexpected(this.peek())
      if (redirects.length > 0) {
        outside({ kind: 'grammar', found: 'assignment with a redirection', why: 'not a command', wayOut: 'a command after the assignment', line })
      }
    }
    return { assignments, words, redirects, line }
  }

  private redirect(token: Extract<Token, { kind: 'op' }>): Redirect | null {
    const target = (): Word => {
      this.next()
      const word = this.peek()
      if (word.kind !== 'word') outside({ kind: 'syntax', found: `${token.op} without a target`, line: token.line })
      this.next()
      return word.word
    }
    switch (token.op) {
      case '>':
        return { kind: 'file', target: 'stdout', mode: 'truncate', path: target(), line: token.line }
      case '>>':
        return { kind: 'file', target: 'stdout', mode: 'append', path: target(), line: token.line }
      case '2>':
        return { kind: 'file', target: 'stderr', mode: 'truncate', path: target(), line: token.line }
      case '2>>':
        return { kind: 'file', target: 'stderr', mode: 'append', path: target(), line: token.line }
      case '&>':
        return { kind: 'file', target: 'both', mode: 'truncate', path: target(), line: token.line }
      case '<':
        return { kind: 'input', path: target(), line: token.line }
      case '<<<':
        return { kind: 'herestring', word: target(), line: token.line }
      case '2>&1':
        this.next()
        return { kind: 'stderr-to-stdout', line: token.line }
      default:
        return null
    }
  }

  private checkReserved(word: Word): void {
    const text = literalText(word)
    if (text !== null && RESERVED.has(text)) {
      const why = text === '!' ? 'negation' : text === 'time' ? 'timing' : 'control flow'
      outside({ kind: 'grammar', found: text, why, wayOut: 'a machine', line: word.line })
    }
  }

  private unexpected(token: Token): never {
    if (token.kind === 'eof') return outside({ kind: 'syntax', found: 'unexpected end of script', line: token.line })
    const found = token.kind === 'op' ? (token.op === 'newline' ? 'newline' : token.op) : token.kind === 'heredoc' ? '<<' : literalText(token.word) ?? 'word'
    return outside({ kind: 'syntax', found: `unexpected ${found}`, line: token.kind === 'word' ? token.word.line : token.line })
  }
}

/** The word's text when it is a single unquoted literal, else null. */
export function literalText(word: Word): string | null {
  if (word.parts.length !== 1) return null
  const part = word.parts[0]!
  return part.kind === 'text' && !part.quoted ? part.text : null
}

/** `NAME=value` with an unquoted name and `=`; the value keeps its own quoting and may start with `~`. */
function asAssignment(word: Word): Assignment | null {
  const first = word.parts[0]
  if (!first || first.kind !== 'text' || first.quoted) return null
  const name = readName(first.text, 0)
  if (name === null || first.text[name.length] !== '=') return null
  const restText = first.text.slice(name.length + 1)
  const parts: WordPart[] = []
  if (restText.length > 0) {
    const after = restText[1] ?? ''
    if (restText[0] === '~' && (after === '' || after === '/')) {
      parts.push({ kind: 'tilde' })
      if (restText.length > 1) parts.push({ kind: 'text', text: restText.slice(1), quoted: false })
    } else {
      parts.push({ kind: 'text', text: restText, quoted: false })
    }
  }
  parts.push(...word.parts.slice(1))
  return { name, value: { parts, line: word.line }, line: word.line }
}
