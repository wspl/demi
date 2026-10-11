import type { JSONContent } from '@tiptap/core'
import { z } from 'zod'
import { ATTACHMENT_MARK, serializeUserMarkdown } from '../../markdown/user-markdown'
import type { MessageCapsule } from '../message-editor/capsules'

// A sent message on the clipboard (`product.md` § Writing a message): its
// Markdown with each file by name for plain text, its formatting for HTML,
// and Demi's own form, which a composer reads to bring the files back as
// capsules without sending their bytes again. Demi's own form travels as a
// JSON attribute of the HTML's outermost element, since browsers keep HTML on
// the clipboard and no format of a page's own.

/** The attribute of the copied HTML's outermost element that holds Demi's own form. */
export const COPIED_MESSAGE_ATTRIBUTE = 'data-demi-message'

/** A file the message holds as a blob of the user's: the upload a paste makes of it sends no bytes. */
const copiedStoredFileSchema = z.object({
  type: z.literal('stored'),
  name: z.string().min(1),
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
  mediaType: z.string(),
  /** The file's size; absent for a picture the message holds without its record. */
  size: z.number().int().nonnegative().optional(),
})

/** A file on a paired device, which the message names by its device and path. */
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

/** The forms of a copy that need no page: Demi's own and plain text, with the lone picture a copy carries as itself. */
export interface MessageClipboard {
  message: CopiedMessage
  /** The Markdown, each file by its name. */
  plain: string
  /** The copy holds one picture and no text: its capsule, whose picture also goes on the clipboard. */
  image: MessageCapsule | null
}

/**
 * The copy of a message, or of a part of it, from its editor document. A
 * capsule whose file the message does not hold yet, such as one still
 * uploading, goes as its name in the text.
 */
export function messageClipboard(doc: JSONContent): MessageClipboard {
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
  return { message: { text, files }, plain, image: lone?.image ? lone : null }
}

/** A capsule's file as Demi's own form holds it; null when the message does not hold it yet. */
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

/** Demi's own form from the attribute's value; null when it is not one, such as HTML from elsewhere. */
export function readCopiedMessage(value: string | null): CopiedMessage | null {
  if (!value) {
    return null
  }
  let json: unknown
  try {
    json = JSON.parse(value)
  } catch {
    // Not JSON: an attribute of the same name some other page wrote.
    return null
  }
  const read = copiedMessageSchema.safeParse(json)
  return read.success ? read.data : null
}

/**
 * How a composer takes the files of a pasted message (`product.md` § Writing
 * a message). A stored file is a capsule at once, and attaches beside it as
 * an upload does: the backend makes an upload of the user's own stored file,
 * sending no bytes. `kept` answers whether the capsule stays: true once the
 * file is attached, or when the paste could not use it and `picture`, the
 * picture the copy carried, uploads in its place under the file's name;
 * false when neither could, and the composer puts the file's name where the
 * capsule stood. A device file is its reference, or null when the composer
 * has it already.
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

/** The picture a copy carried, under its file's name when it is that file's kind, as web browsers name every pasted picture `image.png`. */
function namedPicture(image: File, file: CopiedStoredFile): File {
  return file.mediaType === image.type ? new File([image], file.name, { type: image.type }) : image
}
