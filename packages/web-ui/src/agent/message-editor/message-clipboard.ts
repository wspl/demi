import { DOMSerializer, Fragment, type Schema } from '@tiptap/pm/model'
import { useMediaAddress } from '../media-source'
import { copiedMessageHtml, messageClipboard, type MessageClipboard } from '../message-input/copied-message'
import { capsuleOf, type MessageCapsule } from './capsules'
import { fragmentDocument } from './clipboard-slices'

/**
 * A copy of a message's content in every form: Demi's own and plain text, and
 * its HTML, Demi's own form an attribute of its outermost element; all three
 * hold the same lines, without blank ones at the ends.
 */
export interface MessageCopy {
  clipboard: MessageClipboard
  html: string
}

/**
 * How a message's content goes on the clipboard, from the composer's editor
 * or a sent message alike: its forms, and the clipboard item that holds them
 * with a lone picture as itself.
 */
export function useMessageClipboard() {
  const mediaAddress = useMediaAddress()

  /** Where a capsule's picture loads from, as an address any page can follow; null for a file without one. */
  function pictureAddress(capsule: MessageCapsule): string | null {
    const image = capsule.image
    if (!image) {
      return null
    }
    return new URL(typeof image === 'string' ? image : mediaAddress(image), document.baseURI).href
  }

  /** The message as HTML for mail, documents and chat apps: its formatting, each picture an image, every other file its name. */
  function htmlSerializer(schema: Schema): DOMSerializer {
    const base = DOMSerializer.fromSchema(schema)
    return new DOMSerializer({
      ...base.nodes,
      attachment: (node) => {
        const capsule = capsuleOf(node)
        const src = capsule ? pictureAddress(capsule) : null
        const name = capsule?.name ?? ''
        return src ? ['img', { src, alt: name }] : ['span', name]
      },
    }, base.marks)
  }

  /** `content`, of a document of `schema`, in every form. */
  function forms(schema: Schema, content: Fragment): MessageCopy {
    const clipboard = messageClipboard(fragmentDocument(content))
    const html = htmlSerializer(schema).serializeFragment(Fragment.fromJSON(schema, clipboard.doc.content ?? []))
    return { clipboard, html: copiedMessageHtml(clipboard.message, html) }
  }

  /**
   * The clipboard item of a copy: its forms, and the picture itself when it
   * holds a lone image, so pasting it anywhere pastes the picture.
   */
  function item(copy: MessageCopy): ClipboardItem {
    const image = copy.clipboard.image
    const src = image ? pictureAddress(image) : null
    return new ClipboardItem({
      'text/plain': new Blob([copy.clipboard.plain], { type: 'text/plain' }),
      'text/html': new Blob([copy.html], { type: 'text/html' }),
      ...(src ? { 'image/png': pngOf(src) } : {}),
    })
  }

  return { forms, item }
}

/** A picture as PNG, the one image type every web browser's clipboard takes. */
async function pngOf(src: string): Promise<Blob> {
  const response = await fetch(src, { credentials: 'include' })
  if (!response.ok) {
    throw new Error(`The picture did not load (${response.status}).`)
  }
  const blob = await response.blob()
  if (blob.type === 'image/png') {
    return blob
  }
  const bitmap = await createImageBitmap(blob)
  try {
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height)
    canvas.getContext('2d')?.drawImage(bitmap, 0, 0)
    return await canvas.convertToBlob({ type: 'image/png' })
  } finally {
    bitmap.close()
  }
}
