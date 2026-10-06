import { loadedSize, THUMBNAIL_HEIGHT, thumbnailBox } from '../files/thumbnail'

/**
 * Sizes an image of a run of a message's images (`render.ts`) as a thumbnail
 * (`file-previews.md` § Files named in messages): its box from its size once
 * loaded, the narrowest box before, and its place in the row a thumbnail
 * tall either way, so the row keeps its height when the image loads. An image
 * outside a run keeps the rule for a lone image.
 */
export function fitRunThumbnail(image: HTMLImageElement): void {
  const item = image.closest<HTMLElement>('.media-run > span')
  if (!item)
    return
  const box = thumbnailBox(loadedSize(image))
  image.style.width = `${box.width}px`
  image.style.height = `${box.height}px`
  item.style.height = `${THUMBNAIL_HEIGHT}px`
}

/**
 * Sizes every image of the runs under `root`, after a render replaced them;
 * one whose bytes are not loaded yet is sized again when they are.
 */
export function fitRunThumbnails(root: ParentNode): void {
  for (const image of root.querySelectorAll<HTMLImageElement>('.media-run img'))
    fitRunThumbnail(image)
}
