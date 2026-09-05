/**
 * The backslash escapes of `echo -e` and of printf's format string. The two
 * differ where bash does: echo's `\c` ends the output and its octal form is
 * `\0NNN`; printf's octal form is `\NNN` and it also knows `\"`, `\'`, `\?`.
 */
export function interpretEscapes(text: string, dialect: 'echo' | 'printf'): { text: string; stop: boolean } {
  let out = ''
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]!
    if (ch !== '\\' || i + 1 >= text.length) {
      out += ch
      continue
    }
    const next = text[i + 1]!
    i++
    const simple = SIMPLE[next]
    if (simple !== undefined) {
      out += simple
      continue
    }
    if (next === 'x') {
      const digits = /^[0-9A-Fa-f]{1,2}/.exec(text.slice(i + 1))?.[0]
      if (digits === undefined) {
        out += '\\x'
        continue
      }
      out += String.fromCharCode(parseInt(digits, 16))
      i += digits.length
      continue
    }
    if (dialect === 'echo') {
      if (next === 'c') return { text: out, stop: true }
      if (next === '0') {
        const digits = /^[0-7]{1,3}/.exec(text.slice(i + 1))?.[0] ?? ''
        out += String.fromCharCode(parseInt(digits || '0', 8))
        i += digits.length
        continue
      }
    } else {
      if (next === '"' || next === "'" || next === '?') {
        out += next
        continue
      }
      const octal = /^[0-7]{1,3}/.exec(text.slice(i))?.[0]
      if (octal) {
        out += String.fromCharCode(parseInt(octal, 8) & 0xff)
        i += octal.length - 1
        continue
      }
    }
    out += `\\${next}`
  }
  return { text: out, stop: false }
}

const SIMPLE: Record<string, string> = { a: '\x07', b: '\b', e: '\x1b', E: '\x1b', f: '\f', n: '\n', r: '\r', t: '\t', v: '\v', '\\': '\\' }
