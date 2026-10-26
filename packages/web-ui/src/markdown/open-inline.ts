import type { Token, Tokens } from 'marked'
import { maskMath } from './math-syntax'

/**
 * The formatting a reply leaves open while the rest of it is on its way, shown
 * as it will be once it closes: `say **half wri` shows *half wri* in bold
 * rather than its stars.
 *
 * The reader has already paired every run of marks it can, so a run it left
 * as text on the last line of the last block opens something the reply has
 * not closed yet, when the run can open at all: `max_tokens` holds a `_` that
 * opens nothing. Each such run wraps what follows it in its formatting, after
 * the reading and before the render; the text itself gets no closing marks.
 *
 * Only a part of a reply is read this way; a whole reply renders as it was
 * written, its open marks shown as the text they are.
 */

/** The formatting each run of marks opens, outermost first, or code. */
const OPENERS: readonly [string, readonly ('em' | 'strong' | 'del')[]][] = [
  ['***', ['em', 'strong']],
  ['___', ['em', 'strong']],
  ['**', ['strong']],
  ['__', ['strong']],
  ['~~', ['del']],
  ['*', ['em']],
  ['_', ['em']],
  ['~', ['del']],
]

interface Opener {
  /** Where its run starts in the line's source. */
  at: number
  run: string
  wraps: readonly ('em' | 'strong' | 'del')[] | 'code'
}

const SPACE = /^\s$/u

/**
 * Whether a run of marks between the characters `before` and `after` can
 * open its formatting, as the page's reader decides: `max_tokens` holds a `_`
 * that opens nothing, and `这是**加粗` a `**` that opens bold.
 */
export type CanOpen = (run: string, before: string, after: string) => boolean

/** The character that starts at `at` in `text`, whole: an emoji or a rare CJK character is two units. */
function charAt(text: string, at: number): string {
  const point = text.codePointAt(at)
  return point === undefined ? '' : String.fromCodePoint(point)
}

/** The character that ends at `end` in `text`, whole. */
function charBefore(text: string, end: number): string {
  if (end <= 0)
    return ''
  const low = text.charCodeAt(end - 1)
  const pair = low >= 0xdc00 && low <= 0xdfff && end >= 2
  return charAt(text, end - (pair ? 2 : 1))
}

/**
 * The openers in `line`, the source of the last line of an inline run, among
 * the places `plain` allows: the text the reader left as text. A code span's
 * opener ends them, as what follows it is code. What math holds, closed or
 * still being written, as the `*` of `$a*b`, opens nothing.
 */
function openersIn(line: string, plain: (at: number) => boolean, canOpen: CanOpen): Opener[] {
  const openers: Opener[] = []
  const masked = maskMath(line)
  for (let at = 0; at < line.length;) {
    const char = masked[at]!
    if ((char !== '*' && char !== '_' && char !== '~' && char !== '`') || !plain(at)) {
      at += 1
      continue
    }
    let end = at
    while (masked[end] === char && plain(end)) end += 1
    const run = line.slice(at, end)
    const after = charAt(line, end)
    if (char === '`') {
      // A code span needs no flanking: a run of backticks left unpaired opens one.
      if (after !== '') {
        openers.push({ at, run, wraps: 'code' })
        return openers
      }
    } else {
      const opener = OPENERS.find(([marks]) => marks === run)
      if (opener && after !== '' && canOpen(run, charBefore(line, at), after))
        openers.push({ at, run, wraps: opener[1] })
    }
    at = end
  }
  return openers
}

function textToken(text: string): Tokens.Text {
  return { type: 'text', raw: text, text }
}

/** `tokens` in the formatting of `opener`, its run ahead of them. */
function wrap(opener: Opener, tokens: Token[]): Token[] {
  const text = tokens.map((token) => token.raw).join('')
  if (opener.wraps === 'code')
    return [{ type: 'codespan', raw: opener.run + text, text: text.replace(/\n/g, ' ') }]
  let inner = tokens
  for (const type of opener.wraps.toReversed())
    inner = [{ type, raw: opener.run + text, text, tokens: inner } as Tokens.Em | Tokens.Strong | Tokens.Del]
  return inner
}

/** Whether the reader left `token` as plain text, which a run of marks it holds can open. */
function isPlainText(token: Token): token is Tokens.Text {
  return token.type === 'text' && token.raw === token.text && !token.tokens
}

/**
 * `tokens`, an inline run, with what its last line leaves open wrapped in its
 * formatting; `tokens` itself when nothing is open. Only the last line is
 * read: a run on an earlier line that the reader left as text, such as the
 * `*` of `*args` in prose, stays text, as it most likely will.
 *
 * Each opener wraps the openers after it, built from the innermost out, so
 * the run is read once however many it holds.
 */
function closeOpenInlines(tokens: Token[], canOpen: CanOpen): Token[] {
  const lineStart = tokens.findLastIndex((token) => token.type === 'br') + 1
  // Where each token of the line starts in its source, and the token of each
  // character of it.
  const starts: number[] = []
  const owners: number[] = []
  let line = ''
  for (const token of tokens.slice(lineStart)) {
    starts.push(line.length)
    for (let at = 0; at < token.raw.length; at += 1)
      owners.push(starts.length - 1)
    line += token.raw
  }
  const tokenAt = (at: number) => owners[Math.min(at, owners.length - 1)] ?? 0
  const plain = (at: number) => isPlainText(tokens[lineStart + tokenAt(at)]!)
  const openers = openersIn(line, plain, canOpen)
  if (openers.length === 0)
    return tokens

  /** The tokens of the line from `from` up to `to` in its source, a text token they cut into cut. */
  const between = (from: number, to: number): Token[] => {
    const parts: Token[] = []
    for (let index = tokenAt(from); index < starts.length && starts[index]! < to; index += 1) {
      const token = tokens[lineStart + index]!
      const start = starts[index]!
      const end = start + token.raw.length
      if (start >= from && end <= to)
        parts.push(token)
      else if (isPlainText(token) && Math.max(start, from) < Math.min(end, to))
        parts.push(textToken(token.text.slice(Math.max(from - start, 0), Math.min(to, end) - start)))
    }
    return parts
  }
  let inner: Token[] = []
  let end = line.length
  for (const opener of openers.toReversed()) {
    inner = wrap(opener, [...between(opener.at + opener.run.length, end), ...inner])
    end = opener.at
  }
  return [...tokens.slice(0, lineStart), ...between(0, end), ...inner]
}

/** Whether `row`, a table row's source, ends with a pipe that ends a cell, not a `\|` in its text. */
function endsWithPipe(row: string): boolean {
  const backslashes = /(\\*)\|\s*$/.exec(row)?.[1]
  return backslashes !== undefined && backslashes.length % 2 === 0
}

/**
 * The last block of `tokens`, a part of a reply, with the formatting the last
 * line of its text leaves open wrapped (`closeOpenInlines`): a paragraph's, a
 * heading's, a quote's or a list item's last block's, or the last cell with
 * text of a table's last row. A row that ends in its pipe has ended its last
 * cell.
 */
export function closeOpenBlocks(tokens: Token[], canOpen: CanOpen): void {
  const block = tokens.findLast((token) => token.type !== 'space')
  if (!block)
    return
  switch (block.type) {
    case 'paragraph':
    case 'heading':
    case 'text':
      if (block.tokens) {
        if (block.tokens.some((token) => token.type === 'list'))
          closeOpenBlocks(block.tokens, canOpen)
        else block.tokens = closeOpenInlines(block.tokens, canOpen)
      }
      return
    case 'blockquote':
      closeOpenBlocks(block.tokens ?? [], canOpen)
      return
    case 'list':
      closeOpenBlocks((block as Tokens.List).items.at(-1)?.tokens ?? [], canOpen)
      return
    case 'table': {
      // A table with no row yet has ended its head with the line under it.
      const table = block as Tokens.Table
      if (table.rows.length === 0 || endsWithPipe(table.raw))
        return
      const cell = table.rows.at(-1)!.findLast((each) => each.tokens.length > 0)
      if (cell)
        cell.tokens = closeOpenInlines(cell.tokens, canOpen)
    }
  }
}
