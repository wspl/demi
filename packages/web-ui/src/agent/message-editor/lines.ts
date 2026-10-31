import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import { ATTACHMENT_MARK, lineRoles, type LineRole } from '../../markdown/user-markdown'

// The lines of a message editor's document, a paragraph each, as the dialect
// reads them (`markdown/user-markdown.ts` § The text as typed).

/**
 * The lines of an editor's document, each its text with one character for
 * each capsule, so an offset in it is an offset in the line.
 */
export function lineTexts(doc: ProseMirrorNode): string[] {
  const lines: string[] = []
  doc.forEach((line) => {
    let text = ''
    line.forEach((node) => {
      text += node.isText ? node.text ?? '' : ATTACHMENT_MARK
    })
    lines.push(text)
  })
  return lines
}

/** The role of the line `pos` is in (`lineRoles`). */
export function lineRoleAt(doc: ProseMirrorNode, pos: number): LineRole {
  const index = doc.resolve(pos).index(0)
  return lineRoles(lineTexts(doc))[Math.min(index, doc.childCount - 1)] ?? 'text'
}

