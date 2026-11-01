import { linePrefix, lineRoles, type LinePrefix, type LineRole, type ListMarker } from '../../markdown/user-markdown'

// What Shift-Return, Backspace, Tab and Shift-Tab do on a list item or a quote
// in a message editor (`docs/decisions/0193-a-list-continues-on-shift-return.md`
// tells the rules): each a function of the message's lines (`lineTexts`) and
// the caret's place that answers the change to make, or null where the key
// does what it does on any line. The rules are CodeMirror's Markdown mode's
// (`insertNewlineContinueMarkup`, `deleteMarkupBackward`), written over lines
// rather than a syntax tree, as the highlighting reads a message.
//
//   1. run the tests|   ──Shift-Return──▶   1. run the tests
//   2. fix what fails                       2. |
//                                           3. fix what fails

/** A change to the line `line` as it was: its text from `from` to `to` becomes `insert`. */
export interface LineEdit {
  line: number
  from: number
  to: number
  insert: string
}

/** What a key does to a message's lines. */
export interface ListChange {
  /** Changes to the lines as they were, which never overlap. */
  edits: LineEdit[]
  /** The line `line` splits at `at`, both as they were, and its second half starts with `insert`. */
  split?: { line: number; at: number; insert: string }
  /** Where the caret lands, in the lines as they become; absent where it keeps its place. */
  caret?: { line: number; offset: number }
}

/**
 * A message's lines with how the dialect reads them (`lineRoles`): a line in
 * a fence's code is no list item, whatever it starts with.
 */
interface Message {
  lines: readonly string[]
  roles: LineRole[]
}

function message(lines: readonly string[]): Message {
  return { lines, roles: lineRoles(lines) }
}

/**
 * A line as the lines of a list `depth` quote marks deep read it: where its
 * text starts past those marks (`at`), the spaces after them, and its item,
 * when it is one at that depth. Null for a line quoted less deeply, which no
 * list at that depth holds.
 */
interface Row {
  at: number
  indent: number
  item: LinePrefix | null
  blank: boolean
}

function rowAt(text: Message, line: number, depth: number): Row | null {
  const content = text.lines[line]!
  let at = 0
  for (let mark = 0; mark < depth; mark += 1) {
    const quote = /^[ \t]*>[ \t]?/.exec(content.slice(at))
    if (!quote) {
      return null
    }
    at += quote[0].length
  }
  const prefix = linePrefix(content)
  const item = text.roles[line] === 'text' && prefix.marker && quoteDepth(prefix) === depth ? prefix : null
  return { at, indent: /^[ \t]*/.exec(content.slice(at))![0].length, item, blank: !content.slice(at).trim() }
}

/** Shift-Return at `offset` in the line `index`. */
export function listNewline(lines: readonly string[], index: number, offset: number): ListChange | null {
  const text = message(lines)
  const line = lines[index]!
  const prefix = linePrefix(line)
  if (text.roles[index] !== 'text' || (!prefix.marker && !prefix.quote) || offset < prefix.end) {
    return null
  }
  if (!line.slice(prefix.end).trim()) {
    return endLevel(text, index, prefix)
  }
  const edits: LineEdit[] = []
  // The spaces before the caret stay on neither line, as a code editor's.
  let from = offset
  while (from > prefix.end && /[ \t]/.test(line[from - 1]!)) {
    from -= 1
  }
  if (from < offset) {
    edits.push({ line: index, from, to: offset, insert: '' })
  }
  const { marker } = prefix
  const insert = prefix.quote + prefix.indent + (marker ? nextMarker(marker) : '')
  if (marker?.digits) {
    const number = Number(marker.digits)
    renumber(text, index, prefix, prefix.indent.length, number, number + 1, edits)
  }
  return { edits, split: { line: index, at: offset, insert }, caret: { line: index + 1, offset: insert.length } }
}

/**
 * Shift-Return on an item or a quote line with no text: a nested item moves
 * out to its parent's level with the parent's next marker, any other item
 * loses its marker, and a quote line its last `>`.
 */
function endLevel(text: Message, index: number, prefix: LinePrefix): ListChange {
  const line = text.lines[index]!
  const replace = (content: string): ListChange => ({
    edits: [{ line: index, from: 0, to: line.length, insert: content }],
    caret: { line: index, offset: content.length },
  })
  if (!prefix.marker) {
    return replace(prefix.quote.replace(/>[ \t]?$/, ''))
  }
  const parent = itemAbove(text, index, prefix, (indent) => indent < prefix.indent.length ? 'take' : 'pass')
  if (!parent?.marker) {
    return replace(prefix.quote)
  }
  const change = replace(prefix.quote + parent.indent + nextMarker(parent.marker))
  if (parent.marker.digits) {
    const number = Number(parent.marker.digits)
    renumber(text, index, parent, parent.indent.length, number, number + 1, change.edits)
  }
  return change
}

/**
 * Backspace at `offset` in the line `index`, with no selection: right after a
 * marker, the line continues the item above it at that item's text, or
 * leaves the list when no item is above it; at the text of an item above,
 * with only spaces before the caret, the line goes back to that item's
 * marker; right after a quote's last `>`, that `>` goes.
 */
export function listBackspace(lines: readonly string[], index: number, offset: number): ListChange | null {
  const text = message(lines)
  const line = lines[index]!
  const prefix = linePrefix(line)
  const { marker } = prefix
  if (text.roles[index] !== 'text' || offset !== prefix.end) {
    return null
  }
  const column = prefix.indent.length
  if (marker) {
    const above = itemAbove(text, index, prefix, (indent) => indent === column ? 'take' : indent < column ? 'stop' : 'pass')
    const spaces = above ? ' '.repeat(Math.max(textColumn(above) - column, 0)) : ''
    const edits: LineEdit[] = [{ line: index, from: marker.from, to: prefix.end, insert: spaces }]
    if (marker.digits) {
      const number = Number(marker.digits)
      renumber(text, index, prefix, column, number, number - 1, edits)
    }
    return { edits, caret: { line: index, offset: marker.from + spaces.length } }
  }
  if (prefix.quote && !column) {
    const quote = prefix.quote.replace(/>[ \t]?$/, '')
    return { edits: [{ line: index, from: 0, to: offset, insert: quote }], caret: { line: index, offset: quote.length } }
  }
  const owner = column && itemAbove(text, index, prefix, (indent, item) =>
    !item || indent >= column ? 'pass' : textColumn(item) <= column ? 'take' : 'stop')
  if (owner) {
    const to = prefix.quote.length + owner.indent.length
    return { edits: [{ line: index, from: to, to: offset, insert: '' }], caret: { line: index, offset: to } }
  }
  return null
}

/**
 * Tab (`by` 1) or Shift-Tab (`by` -1) on the list item in the line `index`:
 * the item with its own lines and the items nested under it moves in under
 * the item above it, its marker under that item's text, or out to its
 * parent's level; nothing on a first item or an item at the left edge. Null
 * on a line that is no list item.
 */
export function listIndent(lines: readonly string[], index: number, by: 1 | -1): ListChange | null {
  const text = message(lines)
  const prefix = linePrefix(lines[index]!)
  const { marker } = prefix
  if (text.roles[index] !== 'text' || !marker) {
    return null
  }
  const column = prefix.indent.length
  const unchanged: ListChange = { edits: [] }
  let target: number
  let number: number | undefined
  let parent: LinePrefix | null = null
  if (by > 0) {
    const previous = itemAbove(text, index, prefix, (indent) => indent === column ? 'take' : indent < column ? 'stop' : 'pass')
    if (!previous?.marker) {
      return unchanged
    }
    target = textColumn(previous)
    // Numbered after the last item already at that level under it.
    const sibling = itemAbove(text, index, prefix, (indent) => indent === target ? 'take' : indent <= column ? 'stop' : 'pass')
    number = sibling?.marker?.digits && sibling.marker.delimiter === marker.delimiter ? Number(sibling.marker.digits) + 1 : 1
  } else {
    if (column === 0) {
      return unchanged
    }
    parent = itemAbove(text, index, prefix, (indent) => indent < column ? 'take' : 'pass')
    target = parent ? parent.indent.length : 0
    const above = parent?.marker
    number = above?.digits && above.delimiter === marker.delimiter ? Number(above.digits) + 1 : undefined
  }
  const edits: LineEdit[] = []
  const shift = target - column
  const block = [{ line: index, at: prefix.quote.length, indent: column }, ...linesUnder(text, index, prefix)]
  for (const { line, at, indent } of block) {
    edits.push(shift > 0
      ? { line, from: at, to: at, insert: ' '.repeat(shift) }
      : { line, from: at, to: at + Math.min(-shift, indent), insert: '' })
  }
  if (marker.digits) {
    const own = Number(marker.digits)
    if (number !== undefined && number !== own) {
      edits.push({ line: index, from: marker.from, to: marker.from + marker.digits.length, insert: String(number) })
    }
    if (by > 0) {
      // One item fewer before the ones after it at its old level.
      renumber(text, index, prefix, column, own, own - 1, edits)
    } else {
      // The items after it at its old level now follow its own items moved there.
      const moved = block.slice(1).map(({ line }) => rowAt(text, line, quoteDepth(prefix))!)
        .filter((row) => row.indent + shift === column && row.item?.marker?.digits && row.item.marker.delimiter === marker.delimiter)
        .pop()
      renumber(text, index, prefix, column, own, moved ? Number(moved.item!.marker!.digits) : 0, edits)
      if (parent?.marker?.digits && number !== undefined) {
        renumber(text, index, prefix, target, Number(parent.marker.digits), number, edits)
      }
    }
  }
  return { edits }
}

/** The marker of the item after one with `marker`: the next number, an unchecked box. */
function nextMarker(marker: ListMarker): string {
  const head = marker.digits ? `${Number(marker.digits) + 1}${marker.delimiter}` : marker.bullet!
  return head + (marker.space || ' ') + (marker.task ? `[ ]${marker.task.slice(3) || ' '}` : '')
}

/**
 * The column, after the quote marks, an item's text starts at, past a task's
 * box: where an item nested under it puts its marker.
 */
function textColumn(prefix: LinePrefix): number {
  return prefix.end - prefix.quote.length
}

function quoteDepth(prefix: LinePrefix): number {
  return prefix.quote.split('>').length - 1
}

/**
 * The nearest item above the line `index`, in the list it is in, that
 * `decide` takes by its indentation; a line that is no item passes while it
 * is indented, and `stop`, a blank line or the list's end gives none.
 */
function itemAbove(
  text: Message,
  index: number,
  prefix: LinePrefix,
  decide: (indent: number, item: LinePrefix | null) => 'take' | 'pass' | 'stop',
): LinePrefix | null {
  for (let line = index - 1; line >= 0; line -= 1) {
    const row = rowAt(text, line, quoteDepth(prefix))
    if (!row || row.blank && !row.item) {
      return null
    }
    const verdict = decide(row.indent, row.item)
    if (verdict === 'take' && row.item) {
      return row.item
    }
    if (verdict === 'stop' || !row.item && row.indent === 0) {
      return null
    }
  }
  return null
}

/**
 * The lines after the item in the line `index` that belong to it, its own
 * lines and the items nested under it, each with where its indentation
 * starts and how wide it is.
 */
function linesUnder(text: Message, index: number, prefix: LinePrefix): { line: number; at: number; indent: number }[] {
  const under: { line: number; at: number; indent: number }[] = []
  for (let line = index + 1; line < text.lines.length; line += 1) {
    const row = rowAt(text, line, quoteDepth(prefix))
    if (!row || row.blank && !row.item || row.indent <= prefix.indent.length) {
      break
    }
    under.push({ line, at: row.at, indent: row.indent })
  }
  return under
}

/**
 * Renumbers the ordered items after the line `index` at `column` that ran on
 * by one from `was`, the number before them, so they run on from `now`; it
 * stops at the first that did not, at a bullet, at a line out of the list,
 * and passes the lines nested deeper.
 */
function renumber(text: Message, index: number, prefix: LinePrefix, column: number, was: number, now: number, edits: LineEdit[]): void {
  const delimiter = prefix.marker?.delimiter
  for (let line = index + 1; line < text.lines.length; line += 1) {
    const row = rowAt(text, line, quoteDepth(prefix))
    if (!row || row.blank && !row.item) {
      return
    }
    if (row.indent > column) {
      continue
    }
    const marker = row.item?.marker
    if (row.indent < column || !marker?.digits || marker.delimiter !== delimiter || Number(marker.digits) !== was + 1) {
      return
    }
    was += 1
    now += 1
    if (now !== was) {
      edits.push({ line, from: marker.from, to: marker.from + marker.digits.length, insert: String(now) })
    }
  }
}
