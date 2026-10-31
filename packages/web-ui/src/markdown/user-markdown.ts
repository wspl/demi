import type { JSONContent } from '@tiptap/core'
import { Lexer, type Token } from 'marked'
import { markdownReader } from './reader'

// A user message's Markdown and the document that shows it, both ways, every
// character as typed: a paragraph for each line, and a capsule at each
// attachment mark (`parseTypedMarkdown`).
// Its editors and its bubble draw the same highlighting over the text, from
// how the dialect below reads each line (`markdownSpans`, `lineRoles`,
// `lineForm`; `agent/message-editor/highlights.ts`).
//
// The dialect reads each line's inline constructs on its own, and a fenced
// code block as its lines.

/**
 * Where a file's capsule sits in a message's Markdown; the files follow in the
 * order of their marks. U+FFFC is the object replacement character, which the
 * editor never keeps in text.
 */
export const ATTACHMENT_MARK = '\uFFFC'

const dialect = markdownReader({
  tokenizer: {
    // `_` and `__` stay as typed: `snake_case`, `__init__`.
    emStrong(src) {
      return src.startsWith('_') ? undefined : false
    },
    // HTML, `<…>` and reference links stay as typed.
    tag() {
      return undefined
    },
    autolink() {
      return undefined
    },
    reflink() {
      return undefined
    },
    // A bare URL is a link only with `http` or `https`: not `www.` or an email
    // address.
    url(src) {
      return /^https?:\/\//i.test(src) ? false : undefined
    },
  },
})

function lexLine(src: string): Token[] {
  return src ? Lexer.lexInline(src, dialect.defaults) : []
}

const FENCE_CLOSE = /^ {0,3}(`{3,}|~{3,})[ \t]*$/

/**
 * The fence a line opens a code block with: its indent, its run of backticks or
 * tildes, and the info string after it.
 */
function fenceOf(line: string): { indent: number; fence: string; info: string } | null {
  const open = /^( {0,3})(`{3,}|~{3,})(.*)$/.exec(line)
  // A backtick fence's info string has no backtick: "```a`b" is text.
  if (!open || (open[2]!.startsWith('`') && open[3]!.includes('`'))) {
    return null
  }
  return { indent: open[1]!.length, fence: open[2]!, info: open[3]!.trim() }
}

/** Text as one node, none for no text. */
function textNodes(text: string): JSONContent[] {
  return text ? [{ type: 'text', text }] : []
}

function capsule<Attachment>(attachment: Attachment | undefined): JSONContent[] {
  return attachment === undefined ? [] : [{ type: 'attachment', attrs: { capsule: attachment } }]
}

/**
 * The info string of the code block a line of text opens, or null for a line
 * that opens none.
 */
export function fenceInfo(line: string): string | null {
  return fenceOf(line)?.info ?? null
}

// ── The text as typed ──

/**
 * What a line of a message is: text, a fence that opens or closes a code
 * block, or a line of code between them. A fence closes with a run of the
 * same character at least as long, and a fence left open makes every line
 * after it code, as CommonMark reads fences.
 */
export type LineRole = 'text' | 'fence' | 'code'

/** Each line's role, in the order of `lines`. */
export function lineRoles(lines: readonly string[]): LineRole[] {
  const roles: LineRole[] = []
  for (let index = 0; index < lines.length; index += 1) {
    const open = fenceOf(lines[index]!)
    if (!open) {
      roles.push('text')
      continue
    }
    roles.push('fence')
    for (index += 1; index < lines.length; index += 1) {
      const close = FENCE_CLOSE.exec(lines[index]!)
      if (close && close[1]![0] === open.fence[0] && close[1]!.length >= open.fence.length) {
        roles.push('fence')
        break
      }
      roles.push('code')
    }
  }
  return roles
}

/**
 * What a range of a line is drawn as: the characters of the syntax, dimmed,
 * or text the dialect formats. A link carries its target.
 */
export interface MarkdownSpan {
  from: number
  to: number
  kind: 'syntax' | 'bold' | 'italic' | 'strike' | 'code' | 'link'
  href?: string
}

const FORMAT_KINDS: Partial<Record<string, MarkdownSpan['kind']>> = { strong: 'bold', em: 'italic', del: 'strike' }

/**
 * The ranges of a run of text, a line or its part between two capsules, as
 * the dialect reads it: each construct's text with its kind, and its syntax,
 * which the editor shows dimmed. A link's address is syntax too.
 */
export function markdownSpans(text: string): MarkdownSpan[] {
  const spans: MarkdownSpan[] = []
  const add = (from: number, to: number, kind: MarkdownSpan['kind'], href?: string) => {
    if (to > from) {
      spans.push(href === undefined ? { from, to, kind } : { from, to, kind, href })
    }
  }
  const visit = (tokens: readonly Token[], start: number) => {
    let at = start
    for (const token of tokens) {
      place(token, at)
      at += token.raw.length
    }
  }
  const inner = (token: Token) => ('tokens' in token && token.tokens ? token.tokens.map((each) => each.raw).join('') : '')
  const place = (token: Token, at: number) => {
    const end = at + token.raw.length
    const format = FORMAT_KINDS[token.type]
    if (format) {
      const content = inner(token)
      const open = content ? token.raw.indexOf(content) : Math.floor(token.raw.length / 2)
      add(at, at + open, 'syntax')
      add(at + open, at + open + content.length, format)
      add(at + open + content.length, end, 'syntax')
      visit('tokens' in token ? token.tokens ?? [] : [], at + open)
      return
    }
    switch (token.type) {
      case 'codespan': {
        const ticks = /^`+/.exec(token.raw)?.[0].length ?? 1
        add(at, at + ticks, 'syntax')
        add(at + ticks, end - ticks, 'code')
        add(end - ticks, end, 'syntax')
        return
      }
      case 'link':
      case 'image': {
        if (!token.raw.startsWith('[') && !token.raw.startsWith('!')) {
          // A bare URL: a link as it stands.
          add(at, end, 'link', token.href)
          return
        }
        // The label's text as written, read again where it stands: the
        // lexer's tokens for it hold its escapes undone.
        const lead = token.type === 'image' ? 2 : 1
        const close = labelEnd(token.raw, lead - 1)
        if (close < 0) {
          return
        }
        add(at, at + lead, 'syntax')
        add(at + lead, at + close, 'link', token.href)
        if (token.type === 'link') {
          visit(lexLine(token.raw.slice(lead, close)), at + lead)
        }
        add(at + close, end, 'syntax')
        return
      }
      case 'escape':
        add(at, at + 1, 'syntax')
        return
      default:
    }
  }
  visit(lexLine(text), 0)
  return spans
}

/**
 * What a line of text opens with: a heading, a list item, a quote or a rule,
 * and where its marker (`# `, `- `, `1. `, `> `, with the spaces before it)
 * ends; null for a line of plain text. The editors draw the marker dimmed and
 * the line as its form.
 */
export interface LineForm {
  kind: 'heading' | 'list' | 'quote' | 'rule'
  /** Where the marker ends, and the line's own text starts. */
  markerEnd: number
}

/**
 * CommonMark's line markers, each read on its line alone: a thematic break,
 * an ATX heading, a block quote, and a bullet or an ordered list item at any
 * indentation, so a nested item is one however deep it is.
 */
const LINE_MARKERS: [LineForm['kind'], RegExp][] = [
  ['rule', /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/],
  ['heading', /^ {0,3}#{1,6}(?:[ \t]+|$)/],
  ['quote', /^ {0,3}(?:>[ \t]?)+/],
  ['list', /^[ \t]*(?:[-*+]|\d{1,9}[.)])(?:[ \t]+|$)/],
]

/** The form a line of text opens with (`LineForm`); null for none. */
export function lineForm(line: string): LineForm | null {
  for (const [kind, marker] of LINE_MARKERS) {
    const match = marker.exec(line)
    if (match) {
      return { kind, markerEnd: kind === 'rule' ? line.length : match[0].length }
    }
  }
  return null
}

/**
 * Where the label that opens at `open`, a `[`, closes: the `]` that balances
 * it, past escapes and code spans; -1 for none.
 */
function labelEnd(raw: string, open: number): number {
  let depth = 0
  for (let index = open; index < raw.length; index += 1) {
    const char = raw[index]
    if (char === '\\') {
      index += 1
    } else if (char === '`') {
      const run = /^`+/.exec(raw.slice(index))![0]
      const closing = raw.indexOf(run, index + run.length)
      if (closing >= 0) {
        index = closing + run.length - 1
      }
    } else if (char === '[') {
      depth += 1
    } else if (char === ']') {
      depth -= 1
      if (depth === 0) {
        return index
      }
    }
  }
  return -1
}

/**
 * A message's Markdown as the editors hold it: a paragraph for each line, its
 * text as typed, and the capsule of `attachments[i]` at the i-th attachment
 * mark. A mark with no attachment left is dropped. `serializeTypedMarkdown`
 * writes it back character for character.
 */
export function parseTypedMarkdown<Attachment>(
  markdown: string,
  attachments: readonly Attachment[],
): JSONContent {
  const rest = [...attachments].reverse()
  const lines = markdown.replace(/\r\n?/g, '\n').split('\n')
  return {
    type: 'doc',
    content: lines.map((line) => {
      const content = line.split(ATTACHMENT_MARK).flatMap((segment, position) => [
        ...(position > 0 ? capsule(rest.pop()) : []),
        ...textNodes(segment),
      ])
      return content.length ? { type: 'paragraph', content } : { type: 'paragraph' }
    }),
  }
}

/**
 * The Markdown of an editor's document, character for character, and what
 * its capsules carry, in the order of their marks.
 */
export function serializeTypedMarkdown<Attachment>(
  doc: JSONContent,
): { markdown: string; attachments: Attachment[] } {
  const attachments: Attachment[] = []
  const lines = (doc.content ?? []).map((block) => {
    let line = ''
    for (const node of block.content ?? []) {
      if (node.type === 'attachment') {
        line += ATTACHMENT_MARK
        attachments.push(node.attrs?.['capsule'])
      } else {
        line += (node.text ?? '').replaceAll(ATTACHMENT_MARK, '')
      }
    }
    return line
  })
  return { markdown: lines.join('\n'), attachments }
}
