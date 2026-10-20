import type { JSONContent } from '@tiptap/core'
import { z } from 'zod'
import { jsonOrNull } from '@demicodes/utils'
import { pageLanguage } from '../../i18n/text'
import { t } from '../locales/text'
import { ATTACHMENT_FILE_ATTRIBUTE } from '../../markdown/render'
import { ATTACHMENT_MARK, parseUserMarkdown, serializeUserMarkdown } from '../../markdown/user-markdown'
import type { MessageEditContent } from '../message-editing'
import { contentMessage, type MessageCapsule } from '../message-editor/capsules'

// A sent message on the clipboard: its Markdown with each file by name for
// plain text, its formatting for HTML, and Demi's own form, which a composer
// reads to bring the files back as capsules without sending their bytes again.
// Demi's own form travels as a JSON attribute of the HTML's outermost element,
// since browsers keep HTML on the clipboard and no format of a page's own.

/**
 * The attribute of the copied HTML's outermost element that holds Demi's own
 * form.
 */
export const COPIED_MESSAGE_ATTRIBUTE = 'data-demi-message'

/**
 * A file the message holds as a blob of the user's: the upload a paste makes of
 * it sends no bytes.
 */
export const copiedStoredFileSchema = z.object({
  type: z.literal('stored'),
  name: z.string().min(1),
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
  mediaType: z.string(),
  /**
   * The file's size; absent for a picture the message holds without its record.
   */
  size: z.number().int().nonnegative().optional(),
})

/**
 * A file on a paired device, which the message names by its device and path.
 */
const copiedDeviceFileSchema = z.object({
  type: z.literal('device'),
  name: z.string().min(1),
  deviceId: z.string().min(1),
  /** The device's name, which the capsule shows. */
  host: z.string(),
  path: z.string().min(1),
})

export const copiedFileSchema = z.discriminatedUnion('type', [copiedStoredFileSchema, copiedDeviceFileSchema])
export type CopiedFile = z.infer<typeof copiedFileSchema>
export type CopiedStoredFile = z.infer<typeof copiedStoredFileSchema>
export type CopiedDeviceFile = z.infer<typeof copiedDeviceFileSchema>

/**
 * Demi's own form of a copied message: its Markdown with an attachment mark
 * where each file stands, and each file as the message holds it, in the order
 * of the marks.
 */
export const copiedMessageSchema = z.object({
  text: z.string(),
  files: z.array(copiedFileSchema),
}).refine(
  (message) => message.text.split(ATTACHMENT_MARK).length - 1 === message.files.length,
  { message: 'Each file needs one attachment mark in the text' },
)
export type CopiedMessage = z.infer<typeof copiedMessageSchema>

/**
 * Where a sent message's file is, as the message holds it and its capsule
 * keeps it beside what it shows: the blob of an upload, or the device a file
 * on a paired device is on. Its name, and a device file's host and path, are
 * the capsule's own.
 */
export type FileReference =
  | Omit<CopiedStoredFile, 'name'>
  | Pick<CopiedDeviceFile, 'type' | 'deviceId'>

/**
 * A line of a paragraph that holds nothing but white space: a line break, or
 * white-space text.
 */
function isBlankInline(node: JSONContent): boolean {
  return node.type === 'hardBreak' || (node.type === 'text' && !node.text?.trim())
}

/** A paragraph whose every line is blank. */
function isBlankParagraph(block: JSONContent): boolean {
  return block.type === 'paragraph' && (block.content ?? []).every(isBlankInline)
}

/**
 * The inline content of a paragraph without its blank lines before its
 * first line (`end` false) or after its last (`end` true); a line's
 * indentation stays, since a line with text is not blank.
 */
function trimParagraphLines(content: readonly JSONContent[], end: boolean): JSONContent[] {
  const nodes = end ? content.toReversed() : [...content]
  // Up to the last line break before the first text that is not white space.
  let cut = 0
  for (const [index, node] of nodes.entries()) {
    if (!isBlankInline(node))
      break
    if (node.type === 'hardBreak')
      cut = index + 1
  }
  const kept = nodes.slice(cut)
  return end ? kept.toReversed() : kept
}

/**
 * A document's blocks without the blank lines at its start and its end: blank
 * paragraphs there go, and the first and last paragraphs lose their blank lines
 * on that side; the lines between, and each line's indentation, stay. `from`
 * and `to` are the range of `blocks` kept, so a slice knows whether its open
 * ends were dropped. Code is kept as it is.
 */
export function trimBlankLines(blocks: readonly JSONContent[]): { blocks: JSONContent[]; from: number; to: number } {
  let from = 0
  let to = blocks.length
  while (from < to && isBlankParagraph(blocks[from]!))
    from += 1
  while (to > from && isBlankParagraph(blocks[to - 1]!))
    to -= 1
  const kept = blocks.slice(from, to).map((block, index, all) => {
    if (block.type !== 'paragraph')
      return block
    let content = block.content ?? []
    if (index === 0)
      content = trimParagraphLines(content, false)
    if (index === all.length - 1)
      content = trimParagraphLines(content, true)
    return content.length === (block.content ?? []).length ? block : { ...block, content }
  })
  return { blocks: kept, from, to }
}

/**
 * The forms of a copy that need no page: Demi's own and plain text, with the
 * lone picture a copy carries as itself.
 */
export interface MessageClipboard {
  message: CopiedMessage
  /** The Markdown, each file by its name. */
  plain: string
  /**
   * The copy holds one picture and no text: its capsule, whose picture also
   * goes on the clipboard.
   */
  image: MessageCapsule | null
  /**
   * What the copy holds, without blank lines at its ends: the document its HTML
   * is made from, so every form holds the same lines.
   */
  doc: JSONContent
}

/**
 * The copy of a message, or of a part of it, from its editor document,
 * without blank lines before its first line or after its last. A capsule
 * whose file the message does not hold yet, such as one still uploading,
 * goes as its name in the text.
 */
export function messageClipboard(whole: JSONContent): MessageClipboard {
  const doc = { ...whole, content: trimBlankLines(whole.content ?? []).blocks }
  const { markdown, attachments } = serializeUserMarkdown<MessageCapsule | null>(doc)
  const parts = markdown.split(ATTACHMENT_MARK)
  let text = parts[0] ?? ''
  let plain = text
  const files: CopiedFile[] = []
  attachments.forEach((capsule, index) => {
    const file = capsule ? copiedFile(capsule) : null
    const name = capsule?.name ?? ''
    if (file) {
      text += ATTACHMENT_MARK
      files.push(file)
    } else {
      text += name
    }
    plain += name
    text += parts[index + 1] ?? ''
    plain += parts[index + 1] ?? ''
  })
  const lone = attachments.length === 1 && !parts.join('').trim() ? attachments[0] : null
  return { message: { text, files }, plain, image: lone?.image ? lone : null, doc }
}

/**
 * A message the session holds, from its content, as Demi's own form of a
 * copy of it: what Recall pastes back into the composer, each file a
 * capsule again without its bytes.
 */
export function contentCopy(content: readonly MessageEditContent[]): CopiedMessage {
  const { markdown, capsules } = contentMessage(content)
  return messageClipboard(parseUserMarkdown(markdown, capsules)).message
}

/**
 * A capsule's file as Demi's own form holds it; null when the message does not
 * hold it yet.
 */
function copiedFile(capsule: MessageCapsule): CopiedFile | null {
  const reference = capsule.reference
  if (reference?.type === 'stored') {
    return { ...reference, name: capsule.name }
  }
  if (reference?.type === 'device' && capsule.path) {
    return { ...reference, name: capsule.name, host: capsule.host ?? '', path: capsule.path }
  }
  return null
}

/**
 * Demi's own form from the attribute's value; null when it is not one, such as
 * HTML from elsewhere.
 */
export function readCopiedMessage(value: string | null): CopiedMessage | null {
  if (!value) {
    return null
  }
  const read = copiedMessageSchema.safeParse(jsonOrNull(value))
  return read.success ? read.data : null
}

/**
 * A copy's HTML: `content`, the copy as other apps read it, inside an
 * element whose attribute holds Demi's own form.
 */
export function copiedMessageHtml(message: CopiedMessage, content: Node): string {
  const wrapper = document.createElement('div')
  wrapper.setAttribute(COPIED_MESSAGE_ATTRIBUTE, JSON.stringify(message))
  wrapper.append(content)
  return wrapper.outerHTML
}

/**
 * A drag of an attachment's link or picture in an agent's message carries
 * the file in Demi's own form, as a copy of a message holding it alone
 * would, so a composer it is dropped on makes a capsule of the same file and
 * uploads nothing. The browser's own forms, the address and the picture, stay
 * for any other place.
 */
export function attachmentDrag(event: DragEvent): void {
  const link = event.target instanceof Element ? event.target.closest(`[${ATTACHMENT_FILE_ATTRIBUTE}]`) : null
  const transfer = event.dataTransfer
  if (!link || !transfer) {
    return
  }
  const read = copiedStoredFileSchema.safeParse(jsonOrNull(link.getAttribute(ATTACHMENT_FILE_ATTRIBUTE) ?? ''))
  if (!read.success) {
    return
  }
  const picture = link.querySelector('img')?.cloneNode() ?? document.createTextNode(read.data.name)
  transfer.setData('text/html', copiedMessageHtml({ text: ATTACHMENT_MARK, files: [read.data] }, picture))
}

/**
 * Demi's own form of the message a paste or a drop carries in its HTML; null
 * for HTML from elsewhere.
 */
export function transferCopiedMessage(transfer: DataTransfer): CopiedMessage | null {
  const html = transfer.getData('text/html')
  if (!html.includes(COPIED_MESSAGE_ATTRIBUTE)) {
    return null
  }
  const page = new DOMParser().parseFromString(html, 'text/html')
  return readCopiedMessage(page.querySelector(`[${COPIED_MESSAGE_ATTRIBUTE}]`)?.getAttribute(COPIED_MESSAGE_ATTRIBUTE) ?? null)
}

/**
 * How a composer takes the files of a pasted message. A stored file is a
 * capsule at once, and attaches beside it as an upload does: the backend makes
 * an upload of the user's own stored file, sending no bytes.
 *
 * `kept` answers whether the capsule stays:
 *
 * - true once the file is attached, or when the paste could not use it and
 *   `picture`, the picture the copy carried, uploads in its place under the
 *   file's name
 * - false when neither could, and the composer puts the file's name where the
 *   capsule stood
 *
 * A device file is its reference, or null when the composer cannot take files,
 * as an edit that is no longer editable cannot.
 */
export interface PasteHost {
  claim(file: CopiedStoredFile, picture: File | null): { capsule: MessageCapsule; kept: Promise<boolean> }
  remote(file: CopiedDeviceFile): MessageCapsule | null
}

/**
 * A pasted message as the composer puts it in at once, its text and its
 * capsules, with each stored file's attaching under its capsule's id and the
 * names that stand in the text from the start.
 */
export interface PastedMessage {
  markdown: string
  capsules: MessageCapsule[]
  attaching: { id: string; name: string; kept: Promise<boolean> }[]
  missing: string[]
}

/**
 * Brings a copied message back into a composer: each file a capsule where it
 * stood, at once. A copy of a lone file offers its carried picture for the
 * case the paste cannot use the file.
 */
export function pasteMessage(message: CopiedMessage, image: File | null, host: PasteHost): PastedMessage {
  const parts = message.text.split(ATTACHMENT_MARK)
  let markdown = parts[0] ?? ''
  const capsules: MessageCapsule[] = []
  const attaching: PastedMessage['attaching'] = []
  const missing: string[] = []
  message.files.forEach((file, index) => {
    let capsule: MessageCapsule | null
    if (file.type === 'stored') {
      const picture = image && message.files.length === 1 ? namedPicture(image, file) : null
      const taken = host.claim(file, picture)
      capsule = taken.capsule
      attaching.push({ id: capsule.id, name: file.name, kept: taken.kept })
    } else {
      capsule = host.remote(file)
    }
    if (capsule) {
      markdown += ATTACHMENT_MARK
      capsules.push(capsule)
    } else {
      markdown += file.name
      missing.push(file.name)
    }
    markdown += parts[index + 1] ?? ''
  })
  return { markdown, capsules, attaching, missing }
}

/**
 * The picture a copy carried, under its file's name when it is that file's
 * kind, as web browsers name every pasted picture `image.png`.
 */
function namedPicture(image: File, file: CopiedStoredFile): File {
  return file.mediaType === image.type ? new File([image], file.name, { type: image.type }) : image
}

/**
 * The toast that says which pasted files could not be added, their names
 * pasted in their place; none when every file was.
 */
export function missingFilesToast(names: readonly string[]): { title: string; message: string } | null {
  if (!names.length) {
    return null
  }
  const list = new Intl.ListFormat(pageLanguage(), { type: 'conjunction' }).format(names)
  return {
    title: t('composer.couldntAttach'),
    message: t('attachments.pastedNames', { list }, names.length),
  }
}
