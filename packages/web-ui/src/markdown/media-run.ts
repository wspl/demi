import { declaredSize, loadedSize, tileBox, tileFrame, type PixelSize } from '../files/thumbnail'

/**
 * A message's images and videos, lone and in runs, which stand in blocks of
 * their own (`render.ts`).
 */
const MESSAGE_MEDIA = ':is(.media-lone, .media-run) :is(img, video)'

/**
 * The size a render gave a message's medium from its attachment's record
 * (`render.ts`); null when it gave none.
 */
function renderedSize(element: HTMLElement): PixelSize | null {
  const { width, height } = element.dataset
  if (width === undefined || height === undefined)
    return null
  return declaredSize({ width: Number(width), height: Number(height) })
}

/**
 * How wide the block a message's medium stands in is: the message's width, or a
 * list item's within it.
 */
function blockWidth(element: HTMLElement): number {
  return element.closest<HTMLElement>('.media-lone, .media-run')?.clientWidth ?? 0
}

/**
 * Sizes an image or a video of a message as a tile, in a block `width` pixels
 * wide, by its size once loaded, else by the size its render gave it from its
 * record, which is known before its bytes arrive. An image whose size neither
 * gives yet is held within its tile's frame, which fits it the same way once it
 * loads. A block that is not laid out, 0 pixels wide, leaves it as it is.
 */
export function fitMessageMedium(element: HTMLImageElement | HTMLVideoElement, width = blockWidth(element)): void {
  if (width === 0)
    return
  const box = tileBox(loadedSize(element) ?? renderedSize(element), element instanceof HTMLVideoElement ? 'video' : 'image', width)
  const frame = box ? null : tileFrame(width)
  element.style.width = box ? `${box.width}px` : ''
  element.style.height = box ? `${box.height}px` : ''
  element.style.maxWidth = frame ? `${frame.width}px` : ''
  element.style.maxHeight = frame ? `${frame.height}px` : ''
}

/**
 * Sizes every image and video of the message under `root` as a tile: after
 * a render replaced them, and when the message's width changed. Every
 * block's width is read before any medium changes, so the page lays out
 * once.
 */
export function fitMessageMedia(root: ParentNode): void {
  const media = [...root.querySelectorAll<HTMLImageElement | HTMLVideoElement>(MESSAGE_MEDIA)]
  const widths = media.map(blockWidth)
  media.forEach((element, index) => fitMessageMedium(element, widths[index]))
}

/** A picture of a message, as its preview on hover shows it. */
export interface MessagePicture {
  element: HTMLImageElement | HTMLVideoElement
  kind: 'image' | 'video'
  src: string
  /**
   * Its size from its render, else from its bytes once loaded; null before
   * either.
   */
  size: PixelSize | null
  /** Its alt text. */
  caption: string
}

/**
 * The pictures of the message under `root`, each a tile with its preview on
 * hover.
 */
export function messagePictures(root: ParentNode): MessagePicture[] {
  return [...root.querySelectorAll<HTMLImageElement | HTMLVideoElement>(MESSAGE_MEDIA)].map(messagePicture)
}

/**
 * The preview of a message's picture `element`: its picture, its size, known
 * from its attachment's record before its bytes arrive, and its alt text.
 */
export function messagePicture(element: HTMLImageElement | HTMLVideoElement): MessagePicture {
  const isVideo = element instanceof HTMLVideoElement
  return {
    element,
    kind: isVideo ? 'video' : 'image',
    src: element.getAttribute('src') ?? '',
    size: renderedSize(element) ?? loadedSize(element),
    caption: (isVideo ? element.getAttribute('aria-label') : element.getAttribute('alt')) ?? '',
  }
}
