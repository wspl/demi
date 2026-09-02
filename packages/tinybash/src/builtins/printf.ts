import { outside } from '../outside/reasons'
import type { Builtin } from './io'
import { encodeLatin1, utf8AsLatin1 } from '@demicodes/utils'

/**
 * bash's `printf` for the whitelisted conversions: `%s %c %d %i %u %x %X %o
 * %%` with the flags `-` `+` and space, a width and a precision. The format
 * is reused while arguments remain, a missing argument is empty or zero,
 * and an argument that is not a number is reported as bash reports it.
 * Escapes in the format follow bash (`\NNN` octal, `\xHH`; `\c` is only
 * special inside `%b`, which is outside). Everything else — `-v`, `*` widths, positional `%N$`, `%b`,
 * `%q`, floating conversions, the `#` `0`-with-strings and `'` flags — is
 * outside.
 */

const CONVERSION = /%([-+ 0]*)(\d*)(?:\.(\d*))?([a-zA-Z%])/g

interface Directive {
  flags: string
  width: number
  precision: number | undefined
  conversion: string
}

type Piece = string | Directive

/** The parse-time check: refuses options and any directive outside the whitelist. */
export function printfPaths(argv: readonly string[], line: number): string[] {
  const dashed = argv[0] === '--'
  const format = argv[dashed ? 1 : 0]
  if (format === undefined) return []
  // Without `--`, a format that starts with `-` is an option to bash.
  if (!dashed && format.startsWith('-') && format !== '-') outside({ kind: 'flag', program: 'printf', flag: format, line })
  parseFormat(format, line)
  return []
}

export const printf: Builtin = async (ctx) => {
  let i = 0
  if (ctx.argv[i] === '--') i++
  const format = ctx.argv[i]
  if (format === undefined) {
    await ctx.stderr('bash: line ' + ctx.line + ': printf: usage: printf [-v var] format [arguments]\n')
    return 2
  }
  // Format and arguments are bytes from here on: widths, precisions and `%c` count bytes, as bash does.
  const pieces = parseFormat(utf8AsLatin1(format), ctx.line)
  const args = ctx.argv.slice(i + 1).map(utf8AsLatin1)
  const hasDirective = pieces.some((piece) => typeof piece !== 'string' && piece.conversion !== '%')
  let out = ''
  let index = 0
  let status = 0
  do {
    for (const piece of pieces) {
      if (typeof piece === 'string') {
        out += interpretFormatEscapes(piece)
        continue
      }
      if (piece.conversion === '%') {
        out += '%'
        continue
      }
      const arg = args[index++]
      if (piece.conversion === 's' || piece.conversion === 'c') {
        let text = arg ?? ''
        // bash prints a NUL for `%c` of an empty (or missing) argument.
        if (piece.conversion === 'c') text = text.length > 0 ? text.slice(0, 1) : '\0'
        else if (piece.precision !== undefined) text = text.slice(0, piece.precision)
        out += pad(text, piece.width, piece.flags.includes('-'), false)
        continue
      }
      const number = parseInteger(arg ?? '')
      if (number.error) {
        await ctx.stderr(`bash: line ${ctx.line}: printf: ${arg}: invalid number\n`)
        status = 1
      }
      out += formatInteger(number.value, piece)
    }
  } while (hasDirective && index < args.length)
  await ctx.stdout(encodeLatin1(out))
  return status
}

function parseFormat(format: string, line: number): Piece[] {
  // `\u` / `\U` would need the locale's encoding; outside.
  if (/\\[uU]/.test(format.replace(/\\\\/g, ''))) outside({ kind: 'flag', program: 'printf', flag: '\\u', line })
  const pieces: Piece[] = []
  let last = 0
  CONVERSION.lastIndex = 0
  for (;;) {
    const at = format.indexOf('%', last)
    if (at === -1) break
    if (at > last) pieces.push(format.slice(last, at))
    CONVERSION.lastIndex = at
    const match = CONVERSION.exec(format)
    const spec = match?.index === at ? match : null
    if (!spec) outside({ kind: 'flag', program: 'printf', flag: format.slice(at, at + 3), line })
    const [text, flags, width, precision, conversion] = spec as unknown as [string, string, string, string | undefined, string]
    if (!'scdiuxXo%'.includes(conversion) || flags.includes('0') && (conversion === 's' || conversion === 'c')) {
      outside({ kind: 'flag', program: 'printf', flag: text, line })
    }
    pieces.push({ flags, width: width ? Number(width) : 0, precision: precision === undefined ? undefined : Number(precision || '0'), conversion })
    last = at + text.length
  }
  if (last < format.length) pieces.push(format.slice(last))
  return pieces
}

function pad(text: string, width: number, left: boolean, zero: boolean): string {
  if (text.length >= width) return text
  const fill = width - text.length
  if (left) return text + ' '.repeat(fill)
  if (!zero) return ' '.repeat(fill) + text
  const sign = /^[-+ ]/.test(text) ? text[0]! : ''
  return sign + '0'.repeat(fill) + text.slice(sign.length)
}

function formatInteger(value: bigint, piece: Directive): string {
  const { conversion, flags } = piece
  let digits: string
  let sign = ''
  if (conversion === 'd' || conversion === 'i') {
    const negative = value < 0n
    digits = (negative ? -value : value).toString(10)
    sign = negative ? '-' : flags.includes('+') ? '+' : flags.includes(' ') ? ' ' : ''
  } else {
    const unsigned = value < 0n ? (1n << 64n) + value : value
    digits = conversion === 'u' ? unsigned.toString(10) : conversion === 'o' ? unsigned.toString(8) : unsigned.toString(16)
    if (conversion === 'X') digits = digits.toUpperCase()
  }
  if (piece.precision !== undefined) {
    if (piece.precision === 0 && value === 0n) digits = ''
    else digits = digits.padStart(piece.precision, '0')
  }
  const left = flags.includes('-')
  const zero = flags.includes('0') && !left && piece.precision === undefined
  return pad(sign + digits, piece.width, left, zero)
}

/** strtoimax as bash applies it: a quoted first character is its code, a trailing remainder is an error. */
function parseInteger(arg: string): { value: bigint; error: boolean } {
  if (arg === '') return { value: 0n, error: false }
  if (arg[0] === "'" || arg[0] === '"') {
    return { value: BigInt(arg.codePointAt(1) ?? 0), error: false }
  }
  const match = /^\s*([+-]?)(0[xX][0-9a-fA-F]+|0[0-7]*|[1-9][0-9]*)/.exec(arg)
  if (!match) return { value: 0n, error: true }
  const [text, sign, body] = match
  let magnitude: bigint
  if (/^0[xX]/.test(body!)) magnitude = BigInt(body!)
  else if (body!.length > 1 && body![0] === '0') magnitude = BigInt(`0o${body!.slice(1)}`)
  else magnitude = BigInt(body!)
  const value = sign === '-' ? -magnitude : magnitude
  return { value, error: text.length !== arg.length }
}

/** The escapes bash's printf interprets in its format. */
function interpretFormatEscapes(text: string): string {
  let out = ''
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]!
    if (ch !== '\\' || i + 1 >= text.length) {
      out += ch
      continue
    }
    const next = text[i + 1]!
    i++
    switch (next) {
      case 'a': out += '\x07'; break
      case 'b': out += '\b'; break
      case 'e': case 'E': out += '\x1b'; break
      case 'f': out += '\f'; break
      case 'n': out += '\n'; break
      case 'r': out += '\r'; break
      case 't': out += '\t'; break
      case 'v': out += '\v'; break
      case '\\': out += '\\'; break
      case '"': out += '"'; break
      case "'": out += "'"; break
      case '?': out += '?'; break
      case 'x': {
        const digits = /^[0-9A-Fa-f]{1,2}/.exec(text.slice(i + 1))?.[0]
        if (digits === undefined) {
          out += '\\x'
          break
        }
        out += String.fromCharCode(parseInt(digits, 16))
        i += digits.length
        break
      }
      default: {
        const octal = /^[0-7]{1,3}/.exec(text.slice(i))?.[0]
        if (octal) {
          out += String.fromCharCode(parseInt(octal, 8) & 0xff)
          i += octal.length - 1
          break
        }
        out += `\\${next}`
      }
    }
  }
  return out
}
