import type { EditCopies } from '@demicodes/protocol'
import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'
import type { ReadCallChange } from '@demicodes/web-ui/files/changes'

/**
 * The gallery's stand-in for the backend's blob route: fixture files under
 * made-up blob names, for the media a transcript references. A name the
 * gallery does not hold loads nothing, as a missing blob does.
 */
const blobs = new Map<string, string>()

/** The pixel size of each image and video the gallery holds, as the backend reads it from its header. */
const sizes = new Map<string, { width: number; height: number }>()

function fixture(ref: string, url: string, size?: { width: number; height: number }): string {
  blobs.set(ref, url)
  if (size)
    sizes.set(ref, size)
  return ref
}

/** The fixture media a transcript's specimens reference, by their blob names. */
export const galleryBlobs = {
  /** A 480 × 300 screenshot of the login page. */
  screenshot: fixture('5c'.repeat(32), '/fixtures/preview/photo.png', { width: 480, height: 300 }),
  /** A 360 × 2400 capture of a whole page. */
  fullPage: fixture('9f'.repeat(32), '/fixtures/preview/page-full.png', { width: 360, height: 2400 }),
  /** A 480 × 300 chart. */
  chart: fixture('3d'.repeat(32), '/fixtures/preview/photo-before.png', { width: 480, height: 300 }),
  /** A 48 × 48 icon, smaller than a thumbnail. */
  icon: fixture('4e'.repeat(32), '/fixtures/preview/icon-small.png', { width: 48, height: 48 }),
  /** A 1600 × 300 timeline, wider than a message. */
  timeline: fixture('6b'.repeat(32), '/fixtures/preview/timeline-wide.png', { width: 1600, height: 300 }),
  /** A three-second 320 × 180 recording, as WebM, which every web browser plays. */
  recording: fixture('a7'.repeat(32), '/fixtures/preview/recording.webm', { width: 320, height: 180 }),
  /**
   * A 320 × 180 recording as MP4 whose reference and record carry no size,
   * as when its header was not read: 16:9 until its first frame arrives.
   */
  unsizedRecording: fixture('b8'.repeat(32), '/fixtures/preview/demo.mp4'),
  /** A one-page PDF, for the documents a message carries. */
  guide: fixture('c4'.repeat(32), '/fixtures/preview/guide.pdf'),
}

/**
 * The pixel size a reference to a gallery blob carries, as `width` and
 * `height`; nothing for one whose size is unknown.
 */
export function galleryBlobSize(ref: string): { width?: number; height?: number } {
  return sizes.get(ref) ?? {}
}

/** A blob name the gallery does not hold: its medium cannot load. */
export const missingBlob = '0e'.repeat(32)

/** Where a blob loads from in the gallery; one it does not hold fails to load at once. */
export const galleryBlobUrl: BlobUrl = (ref) => blobs.get(ref) ?? 'data:,'

/**
 * The texts the gallery's blob route holds, such as an edit's two sides,
 * each under one made-up name per text, as content addressing names bytes.
 */
const texts = new Map<string, string>()
const textNames = new Map<string, string>()

/** The made-up blob name of `text`, which the gallery holds from then on. */
function textBlob(text: string): string {
  const known = textNames.get(text)
  if (known !== undefined)
    return known
  const name = (textNames.size + 1).toString(16).padStart(64, 'e')
  textNames.set(text, name)
  texts.set(name, text)
  return name
}

/** The copies of an edit from `original` to `modified`, held by the gallery's blobs. */
export function editCopies(original: string, modified: string): EditCopies {
  return { original: textBlob(original), modified: textBlob(modified) }
}

/**
 * Reads an edit's two sides as the product's change view reads them from
 * the blob route; null when the gallery does not hold one of them.
 */
export const readGalleryEdit: ReadCallChange = async (copies) => {
  const original = texts.get(copies.original)
  const modified = texts.get(copies.modified)
  return original === undefined || modified === undefined ? null : { original, modified }
}
