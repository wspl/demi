import type { JSONContent } from '@tiptap/core'
import { Lexer, Marked, type Token } from 'marked'
import markedCjkFriendly from 'marked-cjk-friendly'

// A user message's Markdown and the editor document that shows it, both ways
// (`product.md` § Writing a message). The message is lines: every line break
// is kept as typed, a fenced code block is the only block, and each line reads
// only the inline constructs the dialect below keeps.

/**
 * Where a file's capsule sits in a message's Markdown; the files follow in the
 * order of their marks. U+FFFC is the object replacement character, which the
 * editor never keeps in text.
 */
export const ATTACHMENT_MARK = '\uFFFC'

const dialect = new Marked({ gfm: true })
// `**注意：**这是` is bold, as a CJK writer means it; plain CommonMark reads the
// delimiters beside CJK punctuation as text.
dialect.use(markedCjkFriendly())
dialect.use({
  tokenizer: {
    // `_` and `__` stay as typed: `snake_case`, `__init__`.
    emStrong(src) {
      return src.startsWith('_') ? undefined : false
    },
    // A single `~` stays as typed: `~/.zshrc`.
    del(src) {
      return src.startsWith('~~') ? false : undefined
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
    // A bare URL is a link only with `http` or `https`: not `www.` or an email address.
    url(src) {
      return /^https?:\/\//i.test(src) ? false : undefined
    },
  },
})

function lexLine(src: string): Token[] {
  return src ? Lexer.lexInline(src, dialect.defaults) : []
}

/** One run of a line as editor nodes. */
function readRun(src: string): JSONContent[] {
  return mergeText(inlineNodes(lexLine(src), []))
}

type Mark = NonNullable<JSONContent['marks']>[number]

/** Marks from the outside in: the order a run of text opens them. */
const MARK_ORDER = ['link', 'bold', 'italic', 'strike', 'code']

const FENCE_CLOSE = /^ {0,3}(`{3,}|~{3,})[ \t]*$/

/** The fence a line opens a code block with: its indent, its run of backticks or tildes, and the info string after it. */
function fenceOf(line: string): { indent: number; fence: string; info: string } | null {
  const open = /^( {0,3})(`{3,}|~{3,})(.*)$/.exec(line)
  // A backtick fence's info string has no backtick: "```a`b" is text.
  if (!open || (open[2]!.startsWith('`') && open[3]!.includes('`'))) {
    return null
  }
  return { indent: open[1]!.length, fence: open[2]!, info: open[3]!.trim() }
}

interface TextBlock {
  kind: 'text'
  lines: string[]
}

interface CodeBlock {
  kind: 'code'
  info: string
  lines: string[]
}

/** A message's lines, cut into runs of text and fenced code blocks; a fence left open runs to the end. */
function splitBlocks(markdown: string): (TextBlock | CodeBlock)[] {
  const lines = markdown.replace(/\r\n?/g, '\n').split('\n')
  const blocks: (TextBlock | CodeBlock)[] = []
  let text: TextBlock | null = null
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index]!
    const open = fenceOf(line)
    if (!open) {
      if (!text) {
        text = { kind: 'text', lines: [] }
        blocks.push(text)
      }
      text.lines.push(line)
      continue
    }
    const code: CodeBlock = { kind: 'code', info: open.info, lines: [] }
    for (index += 1; index < lines.length; index += 1) {
      const close = FENCE_CLOSE.exec(lines[index]!)
      if (close && close[1]![0] === open.fence[0] && close[1]!.length >= open.fence.length) {
        break
      }
      // A fence indented by n spaces takes up to n spaces off each of its lines.
      code.lines.push(lines[index]!.replace(new RegExp(`^ {0,${open.indent}}`), ''))
    }
    blocks.push(code)
    text = null
  }
  return blocks
}

function textNodes(text: string, marks: readonly Mark[]): JSONContent[] {
  if (!text) {
    return []
  }
  return [marks.length ? { type: 'text', text, marks: [...marks] } : { type: 'text', text }]
}

/**
 * The editor nodes for inline tokens under `marks`. A bare URL is text, which
 * the editor shows as a link; a written link is a link mark.
 */
function inlineNodes(tokens: readonly Token[], marks: readonly Mark[]): JSONContent[] {
  return tokens.flatMap((token): JSONContent[] => {
    switch (token.type) {
      case 'strong':
        return inlineNodes(token.tokens ?? [], [...marks, { type: 'bold' }])
      case 'em':
        return inlineNodes(token.tokens ?? [], [...marks, { type: 'italic' }])
      case 'del':
        return inlineNodes(token.tokens ?? [], [...marks, { type: 'strike' }])
      case 'codespan':
        return textNodes(token.text, [...marks, { type: 'code' }])
      case 'link':
        if (!token.raw.startsWith('[')) {
          return textNodes(token.raw, marks)
        }
        return inlineNodes(token.tokens ?? [], [...marks, linkMark(token.href, token.title ?? null)])
      case 'image':
        return [{
          type: 'image',
          attrs: { src: token.href, alt: token.text, title: token.title ?? null },
          ...(marks.length ? { marks: [...marks] } : {}),
        }]
      case 'escape':
      case 'text':
        return textNodes(token.text, marks)
      default:
        return textNodes(token.raw, marks)
    }
  })
}

function linkMark(href: string, title: string | null): Mark {
  return { type: 'link', attrs: { href, title } }
}

/**
 * A message's Markdown as the editor document that shows it. The i-th
 * attachment mark becomes the capsule of `attachments[i]`, which the node
 * keeps whole. A mark with no attachment left is dropped, and an attachment
 * with no mark is not in this message: the marks say which files it has. A
 * mark inside a code block cannot stay there: its capsule follows the block.
 */
export function parseUserMarkdown<Attachment>(
  markdown: string,
  attachments: readonly Attachment[],
): JSONContent {
  const rest = [...attachments].reverse()
  const content: JSONContent[] = []
  for (const block of splitBlocks(markdown)) {
    if (block.kind === 'code') {
      const text = block.lines.join('\n')
      const moved = text.split(ATTACHMENT_MARK).slice(1).flatMap(() => capsule(rest.pop()))
      content.push({
        type: 'codeBlock',
        attrs: { language: block.info || null },
        ...(text ? { content: textNodes(text.replaceAll(ATTACHMENT_MARK, ''), []) } : {}),
      })
      if (moved.length) {
        content.push({ type: 'paragraph', content: moved })
      }
      continue
    }
    const inline = block.lines.flatMap((line, index) => [
      ...(index > 0 ? [{ type: 'hardBreak' }] : []),
      ...line.split(ATTACHMENT_MARK).flatMap((segment, position) => [
        ...(position > 0 ? capsule(rest.pop()) : []),
        ...readRun(segment),
      ]),
    ])
    content.push(inline.length ? { type: 'paragraph', content: inline } : { type: 'paragraph' })
  }
  return { type: 'doc', content }
}

function capsule<Attachment>(attachment: Attachment | undefined): JSONContent[] {
  return attachment === undefined ? [] : [{ type: 'attachment', attrs: { capsule: attachment } }]
}

/**
 * The Markdown that reads back as the editor document, and what its capsules
 * carry, in the order of their marks. Those are the attachments the document
 * was built with: the editor puts nothing else in a capsule.
 */
export function serializeUserMarkdown<Attachment>(
  doc: JSONContent,
): { markdown: string; attachments: Attachment[] } {
  const attachments: Attachment[] = []
  const blocks = (doc.content ?? []).map((block) =>
    block.type === 'codeBlock'
      ? codeBlockMarkdown(block)
      : paragraphMarkdown(block.content ?? [], attachments),
  )
  return { markdown: blocks.join('\n'), attachments }
}

function codeBlockMarkdown(block: JSONContent): string {
  const text = (block.content ?? []).map((node) => node.text ?? '').join('').replaceAll(ATTACHMENT_MARK, '')
  const info = String(block.attrs?.['language'] ?? '')
  // A backtick fence cannot carry a backtick in its info string; a tilde fence can.
  const char = info.includes('`') ? '~' : '`'
  const fence = char.repeat(Math.max(3, longestRun(text, char) + 1))
  return `${fence}${info}\n${text ? `${text}\n` : ''}${fence}`
}

function longestRun(text: string, char: string): number {
  let longest = 0
  let run = 0
  for (const each of text) {
    run = each === char ? run + 1 : 0
    longest = Math.max(longest, run)
  }
  return longest
}

function paragraphMarkdown<Attachment>(content: readonly JSONContent[], attachments: Attachment[]): string {
  const lines: string[] = []
  let line = ''
  let segment: JSONContent[] = []
  const endSegment = () => {
    line += segmentMarkdown(segment)
    segment = []
  }
  const endLine = () => {
    endSegment()
    lines.push(line)
    line = ''
  }
  for (const node of content) {
    if (node.type === 'hardBreak') {
      endLine()
    } else if (node.type === 'attachment') {
      endSegment()
      line += ATTACHMENT_MARK
      attachments.push(node.attrs?.['capsule'])
    } else if (node.type === 'text' && node.text?.includes('\n')) {
      // A line break inside text is a line break all the same.
      node.text.split('\n').forEach((part, index) => {
        if (index > 0) {
          endLine()
        }
        if (part) {
          segment.push({ ...node, text: part })
        }
      })
    } else {
      segment.push(node)
    }
  }
  endLine()
  // A line of text that looks like a fence would open one: its first fence character is escaped.
  return lines.map((each) => {
    const open = fenceOf(each)
    return open ? `${each.slice(0, open.indent)}\\${each.slice(open.indent)}` : each
  }).join('\n')
}

// ── One line's run of inline content, between line breaks and capsules ──

function isFormatting(mark: Mark): boolean {
  return mark.type !== 'code'
}

function markKey(mark: Mark): string {
  if (mark.type !== 'link') {
    return mark.type
  }
  return `link ${JSON.stringify([mark.attrs?.['href'] ?? '', mark.attrs?.['title'] ?? null])}`
}

function sameMarks(a: readonly Mark[] | undefined, b: readonly Mark[] | undefined): boolean {
  const keys = (marks: readonly Mark[] | undefined) => (marks ?? []).map(markKey).sort().join('\n')
  return keys(a) === keys(b)
}

function hasMark(node: JSONContent, mark: Mark): boolean {
  return (node.marks ?? []).some((each) => markKey(each) === markKey(mark))
}

/**
 * The run as Markdown reads it back: text without marks' edge spaces, which
 * move outside their delimiters (`**bold** text`, not `**bold **text`), and
 * neighbouring text of the same marks as one node.
 */
function normalizeRun(nodes: readonly JSONContent[]): JSONContent[] {
  const pieces: JSONContent[] = []
  nodes.forEach((node, index) => {
    if (node.type !== 'text') {
      pieces.push(node)
      return
    }
    const text = (node.text ?? '').replaceAll(ATTACHMENT_MARK, '')
    const marks = node.marks ?? []
    if (!text) {
      return
    }
    if (marks.some((mark) => mark.type === 'code') || !marks.length) {
      pieces.push({ ...node, text })
      return
    }
    const [, lead = '', core = '', trail = ''] = /^(\s*)([\s\S]*?)(\s*)$/.exec(text) ?? []
    const shared = (other: JSONContent | undefined) => marks.filter((mark) => other && hasMark(other, mark))
    pieces.push(...textNodes(lead, shared(nodes[index - 1])))
    pieces.push(...textNodes(core, marks))
    pieces.push(...textNodes(trail, shared(nodes[index + 1])))
  })
  return mergeText(pieces)
}

/** Neighbouring text of the same marks as one node, as the editor keeps it. */
function mergeText(nodes: readonly JSONContent[]): JSONContent[] {
  const merged: JSONContent[] = []
  for (const node of nodes) {
    const last = merged.at(-1)
    if (last?.type === 'text' && node.type === 'text' && sameMarks(last.marks, node.marks)) {
      merged[merged.length - 1] = { ...last, text: `${last.text ?? ''}${node.text ?? ''}` }
    } else {
      merged.push(node)
    }
  }
  return merged
}

function sameRun(a: readonly JSONContent[], b: readonly JSONContent[]): boolean {
  const key = (node: JSONContent) => JSON.stringify([
    node.type,
    node.text ?? null,
    node.type === 'image' ? [node.attrs?.['src'], node.attrs?.['alt'] ?? '', node.attrs?.['title'] ?? null] : null,
    (node.marks ?? []).map(markKey).sort(),
  ])
  return a.length === b.length && a.every((node, index) => key(node) === key(b[index]!))
}

/** Characters the dialect could read as syntax. */
const SYNTAX = /[\\`*~[\]!]/

/**
 * One run of a line as Markdown. Text is written as typed; only when that
 * would read back as something else is each character that could be syntax
 * escaped.
 */
function segmentMarkdown(nodes: readonly JSONContent[]): string {
  const run = normalizeRun(nodes)
  const plain = writeRun(run, false)
  const simple = run.every((node) => node.type === 'text' && !node.marks?.length)
  if (simple && !SYNTAX.test(plain)) {
    return plain
  }
  if (sameRun(normalizeRun(readRun(plain)), run)) {
    return plain
  }
  return writeRun(run, true)
}

/** How many nodes from `index` on carry `mark`. */
function markRun(run: readonly JSONContent[], index: number, mark: Mark): number {
  let end = index
  while (end < run.length && hasMark(run[end]!, mark)) {
    end += 1
  }
  return end - index
}

function writeRun(run: readonly JSONContent[], escape: boolean): string {
  let out = ''
  const open: Mark[] = []
  const close = (count: number) => {
    for (const mark of open.splice(open.length - count).reverse()) {
      out += closing(mark)
    }
  }
  run.forEach((node, index) => {
    const marks = (node.marks ?? []).filter(isFormatting)
    let keep = 0
    while (keep < open.length && marks.some((mark) => markKey(mark) === markKey(open[keep]!))) {
      keep += 1
    }
    close(open.length - keep)
    // A mark that goes on longer opens first, so it can stay open past the shorter ones.
    const opening = marks
      .filter((mark) => !open.some((each) => markKey(each) === markKey(mark)))
      .sort((a, b) =>
        markRun(run, index, b) - markRun(run, index, a)
        || MARK_ORDER.indexOf(a.type) - MARK_ORDER.indexOf(b.type))
    for (const mark of opening) {
      out += mark.type === 'link' ? '[' : delimiter(mark)
      open.push(mark)
    }
    if (node.type === 'image') {
      out += imageMarkdown(node)
    } else if (node.marks?.some((mark) => mark.type === 'code')) {
      out += codeSpan(node.text ?? '')
    } else {
      out += escape ? escapeText(node.text ?? '') : node.text ?? ''
    }
  })
  close(open.length)
  return out
}

function delimiter(mark: Mark): string {
  if (mark.type === 'bold') {
    return '**'
  }
  if (mark.type === 'italic') {
    return '*'
  }
  return '~~'
}

function closing(mark: Mark): string {
  if (mark.type !== 'link') {
    return delimiter(mark)
  }
  return `](${destination(String(mark.attrs?.['href'] ?? ''), mark.attrs?.['title'])})`
}

function destination(target: string, title: unknown): string {
  const written = /[\s()<>]/.test(target) || !target
    ? `<${target.replace(/[\\<>]/g, '\\$&')}>`
    : target.replace(/\\(?=[!-/:-@[-`{-~])/g, '\\\\')
  return typeof title === 'string' && title
    ? `${written} "${title.replace(/["\\]/g, '\\$&')}"`
    : written
}

function imageMarkdown(node: JSONContent): string {
  const alt = String(node.attrs?.['alt'] ?? '').replace(/[\\[\]]/g, '\\$&')
  return `![${alt}](${destination(String(node.attrs?.['src'] ?? ''), node.attrs?.['title'])})`
}

/** Code as written between backticks: a fence longer than any run inside, padded where an edge would be read away. */
function codeSpan(text: string): string {
  const fence = '`'.repeat(longestRun(text, '`') + 1)
  const pad = text.startsWith('`') || text.endsWith('`')
    || (text.startsWith(' ') && text.endsWith(' ') && text.trim() !== '')
    ? ' '
    : ''
  return `${fence}${pad}${text}${pad}${fence}`
}

/**
 * Text with every character escaped that could be syntax: backslashes,
 * backticks, brackets, runs of tildes, and a star unless spaces stand on both
 * sides of it (`2 * 3`). A bare URL is left whole, since an escape in it
 * would stay in the address.
 */
function escapeText(text: string): string {
  const urls = [...text.matchAll(/https?:\/\/[^\s<]*/gi)].map((match) => ({ from: match.index, to: match.index + match[0].length }))
  let out = ''
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]!
    const inUrl = urls.some((url) => index >= url.from && index < url.to)
    const star = char === '*' && !(/\s/.test(text[index - 1] ?? '') && /\s/.test(text[index + 1] ?? ''))
    const tildes = char === '~' && (text[index - 1] === '~' || text[index + 1] === '~')
    out += !inUrl && (star || tildes || /[\\`[\]]/.test(char)) ? `\\${char}` : char
  }
  return out
}

/**
 * What a line's text closes as it is typed: the formatting that ends at the
 * end of `tail`, a run of plain text that follows `before` on its line. The
 * composer replaces the last `length` characters of the tail with `nodes`.
 */
export function closingFormat(
  before: readonly JSONContent[],
  tail: string,
): { length: number; nodes: JSONContent[] } | null {
  const last = lexLine(segmentMarkdown(before) + tail).at(-1)
  if (!last || !tail.endsWith(last.raw)) {
    return null
  }
  // `**bold*` reads as a star and italic, but its writer is one star short of
  // bold: a delimiter typed just before the construct waits for the rest.
  const waiting = ['strong', 'em', 'del'].includes(last.type)
    && tail[tail.length - last.raw.length - 1] === last.raw[0]
  const formats = ['strong', 'em', 'del', 'codespan', 'image'].includes(last.type)
    || (last.type === 'link' && last.raw.startsWith('['))
  return formats && !waiting ? { length: last.raw.length, nodes: mergeText(inlineNodes([last], [])) } : null
}

/** The info string of the code block a line of text opens, or null for a line that opens none. */
export function fenceInfo(line: string): string | null {
  return fenceOf(line)?.info ?? null
}

/** Whether a line of a code block closes it. */
export function closesFence(line: string): boolean {
  return FENCE_CLOSE.test(line)
}

/** The ranges of bare URLs in a run of plain text, which the editor shows as links. */
export function bareUrls(text: string): { from: number; to: number; href: string }[] {
  const ranges: { from: number; to: number; href: string }[] = []
  let offset = 0
  const visit = (tokens: readonly Token[]) => {
    for (const token of tokens) {
      if (token.type === 'link' && !token.raw.startsWith('[')) {
        const from = text.indexOf(token.raw, offset)
        if (from >= 0) {
          ranges.push({ from, to: from + token.raw.length, href: token.href })
          offset = from + token.raw.length
        }
      } else if ('tokens' in token && token.tokens) {
        visit(token.tokens)
      }
    }
  }
  visit(lexLine(text))
  return ranges
}
