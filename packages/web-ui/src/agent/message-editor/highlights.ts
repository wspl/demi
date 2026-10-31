import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { WEB_LINK_REL, isHttpUrl, isLikelyFilePath } from '../../markdown/filePath'
import { codeTextColor, typedCodeRuns } from '../../markdown/highlight'
import { ATTACHMENT_MARK, fenceInfo, lineForm, lineRoles, markdownSpans, type MarkdownSpan } from '../../markdown/user-markdown'
import { lineTexts } from './lines'

// How a user message is highlighted, the one rule both its drawings follow:
// the editor draws it as ProseMirror decorations (`extensions.ts`), the
// bubble as elements (`SentMessage.vue`). A message is lines of plain text,
// every character as typed; highlighting gives a line or a range of it a
// weight, a slant, a strike line, a font or a color, and never a box,
// a well or an indent, so the caret moves through it as through plain text.

/** The attributes a range or a line is drawn with; `nodeName` names the element a range wraps its text in. */
export type HighlightAttributes = Record<string, string>

/** One line's highlighting: the line's own attributes, and its ranges, by offset in the line. */
export interface LineHighlight {
  line: HighlightAttributes | null
  ranges: { from: number; to: number; attributes: HighlightAttributes }[]
}

/**
 * How a link or a bare URL in a message opens: a web address in a new tab, a
 * Host file in the File view, and anything else not at all, shown as text
 * (`message-links.ts`).
 */
export function linkAttributes(target: string): Record<string, string> {
  if (isHttpUrl(target)) {
    return { 'data-link': 'web', 'data-target': target, href: target, target: '_blank', rel: WEB_LINK_REL }
  }
  return { 'data-link': isLikelyFilePath(target) ? 'file' : 'text', 'data-target': target }
}

/** How each kind of range is drawn: the element a reply draws it with, or dimmed. */
function spanAttributes(span: MarkdownSpan): HighlightAttributes {
  switch (span.kind) {
    case 'syntax':
      return { class: 'message-syntax' }
    case 'bold':
      return { nodeName: 'strong' }
    case 'italic':
      return { nodeName: 'em' }
    case 'strike':
      return { nodeName: 's' }
    case 'code':
      return { nodeName: 'code' }
    case 'link':
      return { nodeName: 'a', ...linkAttributes(span.href ?? '') }
  }
}

/**
 * Each line's highlighting, in the order of the document's lines (a paragraph
 * each, a capsule one character):
 *
 * - A heading, a list item, a quote or a rule by its line's form
 *   (`lineForm`), its marker dimmed.
 * - The rest of the line by the dialect's reading of it (`markdownSpans`),
 *   each run between two capsules on its own.
 * - The fences and the code between them in code's monospace, the fences
 *   dimmed and the code in its language's colors.
 */
export function messageHighlights(doc: ProseMirrorNode): LineHighlight[] {
  const texts = lineTexts(doc)
  const roles = lineRoles(texts)
  const lines: LineHighlight[] = texts.map(() => ({ line: null, ranges: [] }))
  let index = 0
  while (index < texts.length) {
    if (roles[index] === 'text') {
      const text = texts[index]!
      const highlight = lines[index]!
      const form = lineForm(text)
      const markerEnd = form?.markerEnd ?? 0
      if (form) {
        highlight.line = { class: `message-${form.kind}` }
        if (markerEnd > 0) {
          highlight.ranges.push({ from: 0, to: markerEnd, attributes: { class: form.kind === 'rule' ? 'message-syntax' : 'message-syntax message-marker' } })
        }
      }
      let at = 0
      for (const run of text.split(ATTACHMENT_MARK)) {
        const skip = Math.max(0, Math.min(run.length, markerEnd - at))
        for (const span of markdownSpans(run.slice(skip))) {
          highlight.ranges.push({ from: at + skip + span.from, to: at + skip + span.to, attributes: spanAttributes(span) })
        }
        at += run.length + 1
      }
      index += 1
      continue
    }
    // A fence, the code after it, and the fence that closes it.
    const open = index
    let end = index + 1
    while (end < texts.length && roles[end] === 'code') {
      end += 1
    }
    const last = end < texts.length && roles[end] === 'fence' ? end : end - 1
    const language = (fenceInfo(texts[open]!) ?? '').split(/\s/)[0] ?? ''
    const runs = typedCodeRuns(texts.slice(open + 1, end).join('\n'), language)
    let offset = 0
    for (let line = open; line <= last; line += 1) {
      const length = texts[line]!.length
      const highlight = lines[line]!
      if (roles[line] === 'fence') {
        highlight.line = { class: 'message-code-line' }
        if (length) {
          highlight.ranges.push({ from: 0, to: length, attributes: { class: 'message-syntax' } })
        }
        continue
      }
      highlight.line = { class: 'message-code-line', style: `color: ${codeTextColor()}` }
      for (const run of runs) {
        const from = Math.max(run.from, offset)
        const to = Math.min(run.to, offset + length)
        if (to > from) {
          highlight.ranges.push({ from: from - offset, to: to - offset, attributes: { class: run.classes } })
        }
      }
      offset += length + 1
    }
    index = last + 1
  }
  return lines
}
