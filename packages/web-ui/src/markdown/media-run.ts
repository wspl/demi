import { declaredSize, loadedSize, thumbnailBox, type PixelSize } from '../files/thumbnail'

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
 * Sizes an image or a video of a message (`file-previews.md` § Files named
 * in messages) by its size once loaded, else by the size its render gave
 * it: in a run, the box of its thumbnail, which the render gave it already
 * when it knew the size, so it moves only when that was unknown; a lone
 * video, the proportions and the width of its first frame, which are 16:9
 * and the message's width until it is known. A lone image keeps the rule
 * for a lone image.
 */
export function fitMessageMedium(element: HTMLImageElement | HTMLVideoElement): void {
  const size = loadedSize(element) ?? renderedSize(element)
  const isVideo = element instanceof HTMLVideoElement
  if (element.closest('.media-run')) {
    const box = thumbnailBox(size, isVideo ? 'video' : 'image')
    element.style.width = `${box.width}px`
    element.style.height = `${box.height}px`
    return
  }
  const wrapper = isVideo ? element.closest<HTMLElement>('.message-video') : null
  if (!wrapper || !size)
    return
  wrapper.style.setProperty('--media-ratio', String(size.width / size.height))
  wrapper.style.setProperty('--media-width', `${size.width}px`)
}

/**
 * Sizes every image of the runs and every video under `root`, after a render
 * replaced them; one whose bytes are not loaded yet is sized again when they
 * are.
 */
export function fitMessageMedia(root: ParentNode): void {
  for (const element of root.querySelectorAll<HTMLImageElement | HTMLVideoElement>('.media-run img, video'))
    fitMessageMedium(element)
}
