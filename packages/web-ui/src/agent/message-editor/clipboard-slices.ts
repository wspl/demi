import type { JSONContent } from '@tiptap/core'
import { Fragment, Slice, type Node as ProseMirrorNode, type Schema } from '@tiptap/pm/model'
import { parseUserMarkdown } from '../../markdown/user-markdown'
import { trimBlankLines } from '../message-input/copied-message'
import type { MessageCapsule } from './capsules'

// What a composer takes from the clipboard and puts on it, as editor slices
// (`product.md` § Writing a message): never a blank line of the clipboard's
// own at either end.

/** How an end of pasted content joins the line it lands in: a paragraph's joins it, a code block stands on its own. */
function opens(node: ProseMirrorNode | null | undefined): number {
  return node?.type.name === 'paragraph' ? 1 : 0
}

/**
 * Pasted or dropped text read as the dialect, without blank lines at its
 * start and end: a single line joins the line at the cursor, code blocks
 * stand on their own.
 */
export function markdownSlice(schema: Schema, text: string, capsules: readonly MessageCapsule[] = []): Slice {
  const parsed = parseUserMarkdown(text, capsules)
  const content = Fragment.fromJSON(schema, trimBlankLines(parsed.content ?? []).blocks)
  return content.childCount ? new Slice(content, opens(content.firstChild), opens(content.lastChild)) : Slice.empty
}

/**
 * A slice the editor copies or takes as it is, from a composer or a message,
 * without blank lines at its start and end. An end whose blocks were
 * dropped joins the line it lands in as `markdownSlice`'s ends do; an end
 * kept stays as open as it was.
 */
export function trimmedSlice(slice: Slice, schema: Schema): Slice {
  const trimmed = trimBlankLines(slice.content.toJSON() ?? [])
  const content = Fragment.fromJSON(schema, trimmed.blocks)
  if (!content.childCount)
    return Slice.empty
  const openStart = trimmed.from > 0 ? opens(content.firstChild) : slice.openStart
  const openEnd = trimmed.to < slice.content.childCount ? opens(content.lastChild) : slice.openEnd
  return new Slice(content, openStart, openEnd)
}

/**
 * Content cut from a document, a slice's or a selection's, as a document of
 * its own: inline content, as ProseMirror leaves a slice within one line,
 * stands in a paragraph.
 */
export function fragmentDocument(content: Fragment): JSONContent {
  const nodes: JSONContent[] = content.toJSON() ?? []
  return { type: 'doc', content: content.firstChild?.isInline ? [{ type: 'paragraph', content: nodes }] : nodes }
}
