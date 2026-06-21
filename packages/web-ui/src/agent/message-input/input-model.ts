import type { UserContentBlock } from '@demi/core'

// The tiptap document the composer edits. With mentions/slash removed it only ever
// contains paragraphs of text, so it maps directly to demi text content blocks.

export interface InputTextNode {
  type: 'text'
  text: string
}

export interface InputParagraph {
  type: 'paragraph'
  content?: InputTextNode[]
}

export interface InputModel {
  type: 'doc'
  content: InputParagraph[]
}

export function docToText(model: InputModel | null | undefined): string {
  if (!model) return ''
  return model.content
    .map((paragraph) => (paragraph.content ?? []).map((node) => node.text).join(''))
    .join('\n')
    .trim()
}

export function docToContent(model: InputModel | null | undefined, attachments: UserContentBlock[] = []): UserContentBlock[] {
  const text = docToText(model)
  const blocks: UserContentBlock[] = text ? [{ type: 'text', text }] : []
  return [...blocks, ...attachments]
}
