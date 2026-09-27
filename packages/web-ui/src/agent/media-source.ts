import { computed, inject, provide, type InjectionKey, type Ref } from 'vue'
import type { DocumentSource, MediaSource as ContentMediaSource, ToolMediaSource } from '@demicodes/protocol'

/**
 * Where the bytes of an image, a video or a document are: an address, or a
 * blob of the user's own, which the backend serves. A transcript never holds
 * a medium's bytes (`runtime.md` § Media).
 */
export type MediaSource = ContentMediaSource | DocumentSource | ToolMediaSource

/**
 * Where the page loads a blob of the user's own, seen as `mediaType`: the
 * product's blob route, or a gallery's fixture (`web-application.md`
 * § Persistence and adapters).
 */
export type BlobUrl = (ref: string, mediaType: string) => string

const blobUrlKey: InjectionKey<BlobUrl> = Symbol('blob-url')

/** Tells every component below where blobs load from; the app's root says it once. */
export function provideBlobUrl(url: BlobUrl): void {
  provide(blobUrlKey, url)
}

/**
 * Where a media source loads from: its URL, or the blob it references. Empty
 * while there is no source.
 */
export function useMediaUrl(source: () => MediaSource | undefined): Readonly<Ref<string>> {
  const blobUrl = inject(blobUrlKey, null)
  if (!blobUrl) {
    throw new Error('Nothing says where blobs load from: the app provides it with provideBlobUrl')
  }
  return computed(() => {
    const value = source()
    if (!value) {
      return ''
    }
    return value.type === 'url' ? value.url : blobUrl(value.ref, value.mediaType)
  })
}
