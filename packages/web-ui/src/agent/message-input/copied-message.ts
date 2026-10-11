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
 * How a composer takes the files of a pasted message. A stored file becomes
 * an upload of the user's own blob, which sends no bytes, and a device file
 * its reference; either answers null when the paste cannot use it, such as a
 * blob of another account or one since deleted. A picture the copy carried
 * is taken as any pasted file is.
 */
export interface PasteHost {
  claim(file: CopiedStoredFile): Promise<MessageCapsule | null>
  remote(file: CopiedDeviceFile): MessageCapsule | null
  add(files: File[]): MessageCapsule[]
}

/** A pasted message as the composer puts it in: its Markdown and capsules, and the names of the files it could not add. */
export interface PastedMessage {
  markdown: string
  capsules: MessageCapsule[]
  missing: string[]
}

/**
 * Brings a copied message back into a composer: each file a capsule where it
 * stood. A file the paste cannot use comes in from `image`, the picture a
 * copy of a lone image carried, and otherwise as its name in the text, which
 * `missing` lists for the composer to say so.
 */
export async function pasteMessage(message: CopiedMessage, image: File | null, host: PasteHost): Promise<PastedMessage> {
  // A claim that failed, as one the backend did not answer, leaves its file
  // out as one the user's blobs do not hold: the toast names it, and the
  // user can attach it again.
  const taken = await Promise.all(message.files.map((file) =>
    file.type === 'stored' ? host.claim(file).catch(() => null) : Promise.resolve(host.remote(file))))
  const parts = message.text.split(ATTACHMENT_MARK)
  let markdown = parts[0] ?? ''
  const capsules: MessageCapsule[] = []
  const missing: string[] = []
  taken.forEach((claimed, index) => {
    const file = message.files[index]!
    const capsule = claimed ?? (image && message.files.length === 1 ? host.add([picture(image, file)])[0] ?? null : null)
    if (capsule) {
      markdown += ATTACHMENT_MARK
      capsules.push(capsule)
    } else {
      markdown += file.name
      missing.push(file.name)
    }
    markdown += parts[index + 1] ?? ''
  })
  return { markdown, capsules, missing }
}

/** The picture a copy carried, under its file's name when it is that file's kind, as web browsers name every pasted picture `image.png`. */
function picture(image: File, file: CopiedFile): File {
  return file.type === 'stored' && file.mediaType === image.type ? new File([image], file.name, { type: image.type }) : image
}
