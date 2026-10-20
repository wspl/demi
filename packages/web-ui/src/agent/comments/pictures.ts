import { inject, provide, type InjectionKey } from 'vue'
import type { MediaSource } from '@demicodes/protocol'
import { copiedStoredFileSchema, type CopiedMessage } from '../message-input/copied-message'
import { ATTACHMENT_FILE_ATTRIBUTE, HOST_FILE_ATTRIBUTE } from '../../markdown/render'
import { ATTACHMENT_MARK } from '../../markdown/user-markdown'
import { pictureQuote } from './anchors'

/**
 * A picture in the agent's reply as a comment, a side question and Send to
 * Conversation take it: the blob of the user's it already is, for an
 * attachment; the blob the page makes of the bytes it shows, for a Host image,
 * whose file may change or go, and for a picture of the web whose site lets the
 * page read it, so every provider reads its bytes; or else the address the
 * picture loads from, which a provider that fetches addresses reads and any
 * other names in a note.
 */

/** A blob of the user's: its name, its SHA-256 and its media type. */
export interface KeptPicture {
  sha256: string
  mediaType: string
}

/**
 * Keeps the bytes the page loads from `src`, a picture named `name`, as a
 * blob of the user's, as an upload does, sending none the user holds
 * already; the product's uploads, or a gallery's fixture.
 */
export type KeepPicture = (src: string, name: string) => Promise<KeptPicture>

const keepPictureKey: InjectionKey<KeepPicture> = Symbol('keepPicture')

export function provideKeepPicture(keep: KeepPicture): void {
  provide(keepPictureKey, keep)
}

export function useKeepPicture(): KeepPicture | null {
  return inject(keepPictureKey, null)
}

/** The attachment `image` shows, as its link names it; null for any other picture. */
function attachmentOf(image: HTMLImageElement): KeptPicture | null {
  const named = image.closest(`[${ATTACHMENT_FILE_ATTRIBUTE}]`)?.getAttribute(ATTACHMENT_FILE_ATTRIBUTE)
  if (!named)
    return null
  const parsed = copiedStoredFileSchema.safeParse(JSON.parse(named))
  return parsed.success ? { sha256: parsed.data.sha256, mediaType: parsed.data.mediaType } : null
}

/** The picture's size as it shows, which a tile of it is laid out by before its bytes arrive. */
function sizeOf(image: HTMLImageElement): { width?: number; height?: number } {
  return image.naturalWidth > 0 ? { width: image.naturalWidth, height: image.naturalHeight } : {}
}

/**
 * Where `image`'s bytes are for the model and the bubble: its attachment's
 * blob, its bytes kept as a blob by `keep`, or, for a picture of the web
 * the page cannot read, its address.
 */
export async function pictureSource(image: HTMLImageElement, keep: KeepPicture | null): Promise<MediaSource> {
  const attachment = attachmentOf(image)
  if (attachment)
    return { type: 'ref', ref: attachment.sha256, mediaType: attachment.mediaType, ...sizeOf(image) }
  const src = image.currentSrc || image.src
  const host = image.hasAttribute(HOST_FILE_ATTRIBUTE)
  if (!keep) {
    if (host)
      throw new Error('This page cannot keep a picture of the Host.')
    return { type: 'url', url: src }
  }
  try {
    const kept = await keep(src, pictureQuote(image))
    return { type: 'ref', ref: kept.sha256, mediaType: kept.mediaType, ...sizeOf(image) }
  } catch (error) {
    // A picture of the web whose site does not let the page read it stays
    // its address; a Host image's bytes have no other way to the model.
    if (host)
      throw error
    return { type: 'url', url: src }
  }
}

/**
 * `image` as Send to Conversation adds it to another conversation's draft:
 * a file of the draft, the blob it is or the page keeps of it; a picture of
 * the web the page cannot read, as one whose site does not let it, goes as
 * its Markdown, which shows it again.
 */
export async function pictureWords(image: HTMLImageElement, keep: KeepPicture | null): Promise<CopiedMessage> {
  const name = pictureQuote(image)
  const file = (kept: KeptPicture): CopiedMessage => ({
    text: ATTACHMENT_MARK,
    files: [{ type: 'stored', name, sha256: kept.sha256, mediaType: kept.mediaType }],
  })
  const attachment = attachmentOf(image)
  if (attachment)
    return file(attachment)
  const src = image.currentSrc || image.src
  if (!keep)
    return { text: `![${name}](${src})`, files: [] }
  try {
    return file(await keep(src, name))
  } catch (error) {
    if (image.hasAttribute(HOST_FILE_ATTRIBUTE))
      throw error
    return { text: `![${name}](${src})`, files: [] }
  }
}
