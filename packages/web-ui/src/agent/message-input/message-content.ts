import { ATTACHMENT_MARK } from '../../markdown/user-markdown'

// A message's content blocks and the Markdown the editor shows them as
// (`product.md` § Attachments): the text blocks are the Markdown, and each
// file stands at a mark between them, in order.

type TextBlock = { type: 'text'; text: string }

function isMedia(block: { type: string } | undefined): boolean {
  return block?.type === 'image' || block?.type === 'video' || block?.type === 'document'
}

/**
 * A message's content as its Markdown, with a mark where each file stands,
 * and its files in the order of their marks. A file is its attachment record
 * with the media block the model reads natively just before it, a media
 * block alone, or a reference. Text blocks that follow one another are
 * paragraphs of one text.
 */
export function splitMessageContent<Block extends { type: string; text?: unknown }>(
  content: readonly Block[],
): { markdown: string; files: Block[][] } {
  let markdown = ''
  const files: Block[][] = []
  content.forEach((block, index) => {
    if (block.type === 'text' && typeof block.text === 'string') {
      markdown += content[index - 1]?.type === 'text' ? `\n\n${block.text}` : block.text
      return
    }
    // A record goes with the media before it.
    const media = files.at(-1)
    if (block.type === 'attachment' && isMedia(content[index - 1]) && media) {
      media.push(block)
      return
    }
    markdown += ATTACHMENT_MARK
    files.push([block])
  })
  return { markdown, files }
}

/**
 * The content a message sends: its text around each file, in the order the
 * composer shows them. The message's first and last text are trimmed, and a
 * text of only spaces between two files is dropped, since a model takes no
 * empty text.
 */
export function joinMessageContent<Block>(
  markdown: string,
  files: readonly (readonly Block[])[],
): (Block | TextBlock)[] {
  const parts = markdown.split(ATTACHMENT_MARK)
  const content: (Block | TextBlock)[] = []
  parts.forEach((part, index) => {
    const first = index === 0 ? part.trimStart() : part
    const text = index === parts.length - 1 ? first.trimEnd() : first
    if (text.trim()) {
      content.push({ type: 'text', text })
    }
    if (index < parts.length - 1) {
      content.push(...(files[index] ?? []))
    }
  })
  // Files without a mark follow the text.
  content.push(...files.slice(parts.length - 1).flat())
  return content
}
