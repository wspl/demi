import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'

/**
 * The gallery's stand-in for the backend's blob route: fixture files under
 * made-up blob names, for the media a transcript references. A name the
 * gallery does not hold loads nothing, as a missing blob does.
 */
const blobs = new Map<string, string>()

function fixture(ref: string, url: string): string {
  blobs.set(ref, url)
  return ref
}

/** The fixture media a transcript's specimens reference, by their blob names. */
export const galleryBlobs = {
  /** A 480 × 300 screenshot of the login page. */
  screenshot: fixture('5c'.repeat(32), '/fixtures/preview/photo.png'),
  /** A 360 × 2400 capture of a whole page. */
  fullPage: fixture('9f'.repeat(32), '/fixtures/preview/page-full.png'),
  /** A 480 × 300 chart. */
  chart: fixture('3d'.repeat(32), '/fixtures/preview/photo-before.png'),
  /** A three-second 320 × 180 recording, as WebM, which every browser plays. */
  recording: fixture('a7'.repeat(32), '/fixtures/preview/recording.webm'),
}

/** A blob name the gallery does not hold: its medium cannot load. */
export const missingBlob = '0e'.repeat(32)

/** Where a blob loads from in the gallery; one it does not hold fails to load at once. */
export const galleryBlobUrl: BlobUrl = (ref) => blobs.get(ref) ?? 'data:,'
