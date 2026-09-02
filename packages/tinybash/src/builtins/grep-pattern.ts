import { outside } from '../outside/reasons'

export type Dialect = 'basic' | 'extended' | 'fixed'

const BRACKET_CLASSES: Record<string, string> = {
  alpha: 'A-Za-z',
  digit: '0-9',
  alnum: 'A-Za-z0-9',
  upper: 'A-Z',
  lower: 'a-z',
  space: ' \\t\\n\\r\\f\\v',
  blank: ' \\t',
  punct: '!-\\/:-@\\[-`{-~',
  print: ' -~',
  graph: '!-~',
  cntrl: '\\x00-\\x1f\\x7f',
  xdigit: '0-9A-Fa-f',
}

function isAsciiLetter(ch: string): boolean {
  return /^[A-Za-z]$/.test(ch)
}

function swapCase(ch: string): string {
  return ch === ch.toLowerCase() ? ch.toUpperCase() : ch.toLowerCase()
}

/** A literal character as a regex atom, both cases when `-i` (ASCII only, as the C locale folds). */
function literal(ch: string, ignoreCase: boolean): string {
  const escaped = /[.*+?^${}()|[\]\\/]/.test(ch) ? `\\${ch}` : ch === '\n' ? '\\n' : ch
  if (ignoreCase && isAsciiLetter(ch)) return `[${ch}${swapCase(ch)}]`
  return escaped
}

/**
 * Translates a POSIX bracket expression body (between `[` and `]`) to a JS
 * class; returns null for the forms without a faithful translation.
 */
function bracket(pattern: string, open: number, ignoreCase: boolean): { source: string; end: number } | null {
  let i = open + 1
  let out = '['
  if (pattern[i] === '^') {
    out += '^'
    i++
  }
  let first = true
  while (i < pattern.length) {
    const ch = pattern[i]!
    if (ch === ']' && !first) return { source: `${out}]`, end: i }
    first = false
    if (ch === '[' && (pattern[i + 1] === ':' || pattern[i + 1] === '=' || pattern[i + 1] === '.')) {
      const kind = pattern[i + 1]!
      const close = pattern.indexOf(`${kind}]`, i + 2)
      if (close === -1) return null
      if (kind !== ':') return null
      const cls = BRACKET_CLASSES[pattern.slice(i + 2, close)]
      if (cls === undefined) return null
      out += cls
      if (ignoreCase && (cls === 'A-Z' || cls === 'a-z')) out += cls === 'A-Z' ? 'a-z' : 'A-Z'
      i = close + 2
      continue
    }
    // A range: both ends single characters (byte order is code-unit order for latin1 text).
    if (pattern[i + 1] === '-' && pattern[i + 2] !== undefined && pattern[i + 2] !== ']') {
      const lo = ch
      const hi = pattern[i + 2]!
      if (lo.charCodeAt(0) > hi.charCodeAt(0)) return null
      out += `${classChar(lo)}-${classChar(hi)}`
      if (ignoreCase && isAsciiLetter(lo) && isAsciiLetter(hi) && (lo === lo.toLowerCase()) === (hi === hi.toLowerCase())) {
        out += `${swapCase(lo)}-${swapCase(hi)}`
      }
      i += 3
      continue
    }
    out += classChar(ch)
    if (ignoreCase && isAsciiLetter(ch)) out += swapCase(ch)
    i++
  }
  return null
}

function classChar(ch: string): string {
  return /[\\\]^[-]/.test(ch) ? `\\${ch}` : ch === '\n' ? '\\n' : ch
}

/**
 * A grep pattern to a JS regex source, or throws `OutsideError` when the
 * dialect has no faithful translation (back-references, BRE groups and
 * intervals, collating elements, unknown escapes).
 */
export function translatePattern(pattern: string, dialect: Dialect, ignoreCase: boolean, line: number): RegExp {
  if (pattern.includes('\n')) outside({ kind: 'pattern', pattern, line })
  const refuse = (): never => outside({ kind: 'pattern', pattern, line })
  if (dialect === 'fixed') {
    return new RegExp([...pattern].map((ch) => literal(ch, ignoreCase)).join(''))
  }
  const ere = dialect === 'extended'
  let out = ''
  // Whether a repetition operator here would apply to nothing (start, after `(` or `|`): GNU takes it literally.
  let atStart = true
  let depth = 0
  for (let i = 0; i < pattern.length; i++) {
    const ch = pattern[i]!
    if (ch === '\\') {
      const next = pattern[i + 1]
      if (next === undefined) refuse()
      i++
      if (ere) {
        if (/[0-9]/.test(next!)) refuse()
        if ('wWsSbB'.includes(next!)) {
          out += `\\${next}`
        } else if (next === '<') out += '\\b(?=\\w)'
        else if (next === '>') out += '\\b(?<=\\w)'
        else if (/[.[\]*+?{}()|^$\\\/]/.test(next!)) out += `\\${next}`
        else if (/[A-Za-z]/.test(next!)) refuse()
        else out += literal(next!, ignoreCase)
        atStart = false
        continue
      }
      // BRE: the GNU operators spelled with a backslash.
      if (next === '(' || next === ')' || next === '{' || next === '}' || /[0-9]/.test(next!)) refuse()
      if (next === '|') {
        out += '|'
        atStart = true
        continue
      }
      if (next === '+' || next === '?') {
        out += atStart ? `\\${next}` : next
        atStart = false
        continue
      }
      if ('wWsSbB'.includes(next!)) out += `\\${next}`
      else if (next === '<') out += '\\b(?=\\w)'
      else if (next === '>') out += '\\b(?<=\\w)'
      else if (/[A-Za-z]/.test(next!)) refuse()
      else out += literal(next!, ignoreCase)
      atStart = false
      continue
    }
    if (ch === '[') {
      const result = bracket(pattern, i, ignoreCase)
      if (result === null) refuse()
      out += result!.source
      i = result!.end
      atStart = false
      continue
    }
    if (ch === '.') {
      out += '.'
      atStart = false
      continue
    }
    if (ch === '*') {
      out += atStart ? '\\*' : '*'
      atStart = false
      continue
    }
    if (ch === '^') {
      // BRE: an anchor only at the start of the pattern or after `\(` / `\|`; ERE: an anchor anywhere.
      out += ere || atStart ? '^' : '\\^'
      continue
    }
    if (ch === '$') {
      const last = i === pattern.length - 1 || (!ere ? false : pattern[i + 1] === '|' || pattern[i + 1] === ')')
      out += ere || last ? '$' : '\\$'
      atStart = false
      continue
    }
    if (ere) {
      if (ch === '(') {
        out += '('
        depth++
        atStart = true
        continue
      }
      if (ch === ')') {
        if (depth === 0) {
          out += '\\)'
        } else {
          out += ')'
          depth--
        }
        atStart = false
        continue
      }
      if (ch === '|') {
        out += '|'
        atStart = true
        continue
      }
      if (ch === '+' || ch === '?') {
        out += atStart ? `\\${ch}` : ch
        atStart = false
        continue
      }
      if (ch === '{') {
        const interval = /^\{(\d+)(,(\d*))?\}/.exec(pattern.slice(i))
        if (!interval || atStart) refuse()
        out += interval![0]
        i += interval![0].length - 1
        atStart = false
        continue
      }
      if (ch === '}') {
        out += '\\}'
        atStart = false
        continue
      }
    } else if (ch === '{' || ch === '}' || ch === '(' || ch === ')' || ch === '+' || ch === '?' || ch === '|') {
      out += `\\${ch}`
      atStart = false
      continue
    }
    out += literal(ch, ignoreCase)
    atStart = false
  }
  if (depth !== 0) refuse()
  try {
    return new RegExp(out)
  } catch {
    return refuse()
  }
}
