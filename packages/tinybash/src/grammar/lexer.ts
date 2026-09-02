import type { HeredocBody, Word, WordPart } from './ast'
import { outside } from '../outside/reasons'

export type Operator = ';' | 'newline' | '&&' | '||' | '|' | '>' | '>>' | '<' | '2>' | '2>>' | '2>&1' | '&>' | '<<<'

export type Token =
  | { kind: 'word'; word: Word }
  | { kind: 'op'; op: Operator; line: number }
  /** `<<` / `<<-` with its delimiter; the body is filled in once the line ends. */
  | { kind: 'heredoc'; strip: boolean; delimiter: string; quoted: boolean; body: HeredocBody | null; line: number }
  | { kind: 'eof'; line: number }

const NAME_START = /[A-Za-z_]/
const NAME_CHAR = /[A-Za-z0-9_]/

/** Reads `NAME` at `pos`, or null when no name starts there. */
export function readName(text: string, pos: number): string | null {
  if (pos >= text.length || !NAME_START.test(text[pos]!)) return null
  let end = pos + 1
  while (end < text.length && NAME_CHAR.test(text[end]!)) end++
  return text.slice(pos, end)
}

/**
 * Turns a script into tokens. Everything the grammar does not admit throws
 * `OutsideError` here, with the line it was found on.
 */
export function tokenize(script: string): Token[] {
  return new Lexer(script).run()
}

class Lexer {
  private pos = 0
  private line = 1
  private readonly tokens: Token[] = []
  private readonly pendingHeredocs: Extract<Token, { kind: 'heredoc' }>[] = []

  constructor(private readonly text: string) {}

  run(): Token[] {
    if (this.text.includes('\r')) {
      const line = this.text.slice(0, this.text.indexOf('\r')).split('\n').length
      outside({ kind: 'syntax', found: 'carriage return', line })
    }
    while (this.pos < this.text.length) {
      const ch = this.text[this.pos]!
      if (ch === ' ' || ch === '\t') {
        this.pos++
      } else if (ch === '\n') {
        this.pos++
        this.tokens.push({ kind: 'op', op: 'newline', line: this.line })
        this.line++
        this.readHeredocBodies()
      } else if (ch === '\\' && this.text[this.pos + 1] === '\n') {
        this.pos += 2
        this.line++
      } else if (ch === '#') {
        while (this.pos < this.text.length && this.text[this.pos] !== '\n') this.pos++
      } else if (this.readOperator()) {
        // consumed
      } else {
        this.tokens.push({ kind: 'word', word: this.readWord() })
      }
    }
    if (this.pendingHeredocs.length > 0) {
      outside({ kind: 'syntax', found: `here-document delimited by end-of-file (wanted ${this.pendingHeredocs[0]!.delimiter})`, line: this.line })
    }
    this.tokens.push({ kind: 'eof', line: this.line })
    return this.tokens
  }

  private peek(offset = 0): string {
    return this.text[this.pos + offset] ?? ''
  }

  private grammar(found: string, why: string, wayOut: string): never {
    return outside({ kind: 'grammar', found, why, wayOut, line: this.line })
  }

  private op(op: Operator, length: number): true {
    this.tokens.push({ kind: 'op', op, line: this.line })
    this.pos += length
    return true
  }

  /** Operators, including the refused ones; returns false when a word starts here. */
  private readOperator(): boolean {
    const a = this.peek()
    const b = this.peek(1)
    const c = this.peek(2)
    switch (a) {
      case ';':
        if (b === ';') this.grammar(';;', 'case syntax', 'a machine')
        return this.op(';', 1)
      case '&':
        if (b === '&') return this.op('&&', 2)
        if (b === '>') {
          if (c === '>') this.grammar('&>>', 'redirection form not available here', '>> file 2>&1')
          return this.op('&>', 2)
        }
        return this.grammar('&', 'background jobs and job control', 'a machine')
      case '|':
        if (b === '|') return this.op('||', 2)
        if (b === '&') this.grammar('|&', 'redirection form not available here', '2>&1 |')
        return this.op('|', 1)
      case '(':
        return this.grammar('(', 'subshells and grouping', 'a machine')
      case ')':
        return this.grammar(')', 'subshells and grouping', 'a machine')
      case '<':
        if (b === '<') {
          if (c === '<') return this.op('<<<', 3)
          return this.readHeredocOperator()
        }
        if (b === '(') this.grammar('<(', 'process substitution', 'a machine')
        if (b === '&') this.grammar('<&', 'redirection form not available here', 'a machine')
        if (b === '>') this.grammar('<>', 'redirection form not available here', 'a machine')
        return this.op('<', 1)
      case '>':
        if (b === '>') return this.op('>>', 2)
        if (b === '(') this.grammar('>(', 'process substitution', 'a machine')
        if (b === '&') this.grammar('>&', 'redirection form not available here', '2>&1, or a machine')
        if (b === '|') this.grammar('>|', 'redirection form not available here', '> file')
        return this.op('>', 1)
      default:
        break
    }
    // `N>` with the digit directly before the operator is a descriptor number.
    if (/[0-9]/.test(a) && (b === '>' || b === '<') && this.atWordStart()) {
      if (a !== '2' || b !== '>') this.grammar(`${a}${b}`, 'redirection form not available here', '2>, 2>>, 2>&1, or a machine')
      if (c === '>') return this.op('2>>', 3)
      if (c === '&') {
        if (this.peek(3) === '1') return this.op('2>&1', 4)
        this.grammar(`2>&${this.peek(3)}`, 'redirection form not available here', '2>&1, or a machine')
      }
      return this.op('2>', 2)
    }
    return false
  }

  private atWordStart(): boolean {
    const prev = this.text[this.pos - 1]
    return prev === undefined || prev === ' ' || prev === '\t' || prev === '\n' || prev === ';' || prev === '|' || prev === '&'
  }

  private readHeredocOperator(): true {
    const line = this.line
    let strip = false
    this.pos += 2
    if (this.peek() === '-') {
      strip = true
      this.pos++
    }
    while (this.peek() === ' ' || this.peek() === '\t') this.pos++
    if (this.pos >= this.text.length || this.peek() === '\n') {
      outside({ kind: 'syntax', found: '<< without a delimiter', line })
    }
    const word = this.readWord()
    let delimiter = ''
    let quoted = false
    for (const part of word.parts) {
      if (part.kind === 'text') {
        delimiter += part.text
        if (part.quoted) quoted = true
      } else if (part.kind === 'param') {
        delimiter += `$${part.name}`
      } else {
        delimiter += '~'
      }
    }
    const token: Extract<Token, { kind: 'heredoc' }> = { kind: 'heredoc', strip, delimiter, quoted, body: null, line }
    this.tokens.push(token)
    this.pendingHeredocs.push(token)
    return true
  }

  /** After a newline: the bodies of every heredoc opened on the line just ended, in order. */
  private readHeredocBodies(): void {
    while (this.pendingHeredocs.length > 0) {
      const heredoc = this.pendingHeredocs.shift()!
      const lines: string[] = []
      let closed = false
      while (this.pos < this.text.length) {
        let end = this.text.indexOf('\n', this.pos)
        if (end === -1) end = this.text.length
        let lineText = this.text.slice(this.pos, end)
        this.pos = Math.min(end + 1, this.text.length)
        this.line++
        if (heredoc.strip) lineText = lineText.replace(/^\t+/, '')
        if (lineText === heredoc.delimiter) {
          closed = true
          break
        }
        lines.push(lineText)
      }
      if (!closed) {
        outside({ kind: 'syntax', found: `here-document delimited by end-of-file (wanted ${heredoc.delimiter})`, line: heredoc.line })
      }
      const text = lines.length > 0 ? `${lines.join('\n')}\n` : ''
      heredoc.body = heredoc.quoted ? { kind: 'literal', text } : { kind: 'expand', parts: this.heredocParts(text, heredoc.line) }
    }
  }

  /** A bare-delimiter body: `$NAME`, `${NAME}` and the `\$` `\\` `` \` `` escapes, as bash. */
  private heredocParts(text: string, line: number): WordPart[] {
    const parts: WordPart[] = []
    let buffer = ''
    const flush = () => {
      if (buffer.length > 0) parts.push({ kind: 'text', text: buffer, quoted: true })
      buffer = ''
    }
    let i = 0
    while (i < text.length) {
      const ch = text[i]!
      if (ch === '\\' && i + 1 < text.length) {
        const next = text[i + 1]!
        if (next === '$' || next === '\\' || next === '`') {
          buffer += next
          i += 2
          continue
        }
        if (next === '\n') {
          i += 2
          continue
        }
        buffer += ch
        i++
        continue
      }
      if (ch === '`') outside({ kind: 'grammar', found: '`', why: 'command substitution', wayOut: 'write the value literally; a machine for the rest', line })
      if (ch === '$') {
        const consumed = this.dollar(text, i, true, parts, flush, line)
        if (consumed > 0) {
          i += consumed
          continue
        }
      }
      buffer += ch
      i++
    }
    flush()
    return parts
  }

  /**
   * `$` at `text[i]`: pushes a param part and returns the characters consumed,
   * throws for the refused forms, or returns 0 when the `$` is literal.
   */
  private dollar(text: string, i: number, quoted: boolean, parts: WordPart[], flush: () => void, line: number): number {
    const next = text[i + 1] ?? ''
    if (next === '(') {
      const found = text[i + 2] === '(' ? '$((' : '$('
      const why = found === '$((' ? 'arithmetic substitution' : 'command substitution'
      outside({ kind: 'grammar', found, why, wayOut: 'write the value literally; a machine for the rest', line })
    }
    if (next === '{') {
      const name = readName(text, i + 2)
      if (name !== null && text[i + 2 + name.length] === '}') {
        flush()
        parts.push({ kind: 'param', name, quoted })
        return name.length + 3
      }
      const close = text.indexOf('}', i)
      const found = close === -1 ? text.slice(i, i + 12) : text.slice(i, close + 1)
      outside({ kind: 'grammar', found, why: 'parameter operators', wayOut: 'a plain $NAME or the value literally', line })
    }
    const name = readName(text, i + 1)
    if (name !== null) {
      flush()
      parts.push({ kind: 'param', name, quoted })
      return name.length + 1
    }
    if (/[0-9?@*#$!-]/.test(next)) {
      outside({ kind: 'grammar', found: `$${next}`, why: 'positional and special parameters', wayOut: 'the value literally', line })
    }
    return 0
  }

  private readWord(): Word {
    const line = this.line
    const parts: WordPart[] = []
    let buffer = ''
    let bufferQuoted = false
    const flush = () => {
      if (buffer.length > 0) parts.push({ kind: 'text', text: buffer, quoted: bufferQuoted })
      buffer = ''
    }
    const put = (ch: string, quoted: boolean) => {
      if (buffer.length > 0 && bufferQuoted !== quoted) flush()
      bufferQuoted = quoted
      buffer += ch
    }
    const text = this.text
    // A leading unquoted `~` alone or before `/` is the home; `~name` is refused.
    if (text[this.pos] === '~') {
      const after = text[this.pos + 1] ?? ''
      if (after === '/' || after === '' || this.isWordEnd(after)) {
        parts.push({ kind: 'tilde' })
        this.pos++
      } else if (NAME_START.test(after)) {
        const name = readName(text, this.pos + 1)!
        this.grammar(`~${name}`, 'tilde with a user name', "the user's home path written out")
      }
    }
    while (this.pos < text.length) {
      const ch = text[this.pos]!
      if (this.isWordEnd(ch)) break
      if (ch === "'") {
        const close = text.indexOf("'", this.pos + 1)
        if (close === -1) outside({ kind: 'syntax', found: "unterminated '", line })
        const inner = text.slice(this.pos + 1, close)
        this.line += inner.split('\n').length - 1
        // An empty pair still makes the word exist (`''` is an empty argument).
        if (inner.length === 0 && parts.length === 0 && buffer.length === 0) parts.push({ kind: 'text', text: '', quoted: true })
        for (const c of inner) put(c, true)
        this.pos = close + 1
        continue
      }
      if (ch === '"') {
        this.pos++
        let closed = false
        let sawContent = false
        while (this.pos < text.length) {
          const c = text[this.pos]!
          if (c === '"') {
            closed = true
            this.pos++
            break
          }
          sawContent = true
          if (c === '\\') {
            const next = text[this.pos + 1] ?? ''
            if (next === '"' || next === '\\' || next === '$' || next === '`') {
              put(next, true)
              this.pos += 2
              continue
            }
            if (next === '\n') {
              this.pos += 2
              this.line++
              continue
            }
            put(c, true)
            this.pos++
            continue
          }
          if (c === '`') this.grammar('`', 'command substitution', 'write the value literally; a machine for the rest')
          if (c === '$') {
            const consumed = this.dollar(text, this.pos, true, parts, flush, this.line)
            if (consumed > 0) {
              this.pos += consumed
              continue
            }
          }
          if (c === '\n') this.line++
          put(c, true)
          this.pos++
        }
        if (!closed) outside({ kind: 'syntax', found: 'unterminated "', line })
        if (!sawContent && parts.length === 0 && buffer.length === 0) parts.push({ kind: 'text', text: '', quoted: true })
        continue
      }
      if (ch === '\\') {
        const next = text[this.pos + 1]
        if (next === undefined) outside({ kind: 'syntax', found: 'trailing backslash', line })
        if (next === '\n') {
          this.pos += 2
          this.line++
          continue
        }
        put(next, true)
        this.pos += 2
        continue
      }
      if (ch === '`') this.grammar('`', 'command substitution', 'write the value literally; a machine for the rest')
      if (ch === '$') {
        const consumed = this.dollar(text, this.pos, false, parts, flush, this.line)
        if (consumed > 0) {
          this.pos += consumed
          continue
        }
      }
      if (ch === '{' || ch === '}') {
        const lone = parts.length === 0 && buffer.length === 0 && this.isWordEnd(text[this.pos + 1] ?? '')
        if (lone) this.grammar(ch, 'grouping', 'a machine')
        if (ch === '{' && this.isBraceExpansion()) this.grammar(this.braceText(), 'brace expansion is not expanded here', 'list the names')
      }
      put(ch, false)
      this.pos++
    }
    flush()
    return { parts, line }
  }

  /** bash expands `{a,b}` and `{1..3}`; `{}` and `{x}` stay literal. */
  private isBraceExpansion(): boolean {
    const inner = this.braceText().slice(1, -1)
    return inner.includes(',') || inner.includes('..')
  }

  private braceText(): string {
    let i = this.pos + 1
    let depth = 1
    while (i < this.text.length && !this.isWordEnd(this.text[i]!)) {
      if (this.text[i] === '{') depth++
      if (this.text[i] === '}' && --depth === 0) return this.text.slice(this.pos, i + 1)
      i++
    }
    return this.text.slice(this.pos, i)
  }

  private isWordEnd(ch: string): boolean {
    return ch === '' || ch === ' ' || ch === '\t' || ch === '\n' || ch === ';' || ch === '&' || ch === '|' || ch === '<' || ch === '>' || ch === '(' || ch === ')'
  }
}
