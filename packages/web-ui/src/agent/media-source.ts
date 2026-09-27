import { inject, provide, ref, watch, type InjectionKey, type Ref } from 'vue'
import type { DocumentSource, MediaSource as ContentMediaSource, ToolMediaSource } from '@demicodes/protocol'

/**
 * Where the bytes of an image, a video or a document are: an address, a blob
 * of the user's own, which the backend serves, or the bytes themselves.
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
 * Where a media source loads from: its URL, the blob it references, or its
 * bytes as an object URL, which is released when the source changes or the
 * caller's scope ends. Empty while there is no source.
 */
export function useMediaUrl(source: () => MediaSource | undefined): Readonly<Ref<string>> {
  const blobUrl = inject(blobUrlKey, null)
  if (!blobUrl) {
    throw new Error('Nothing says where blobs load from: the app provides it with provideBlobUrl')
  }
  const url = ref('')
  watch(
    source,
    (value, _previous, cleanup) => {
      if (!value) {
        url.value = ''
        return
      }
      if (value.type === 'url') {
        url.value = value.url
        return
      }
      if (value.type === 'ref') {
        url.value = blobUrl(value.ref, value.mediaType)
        return
      }
      const bytes = Uint8Array.from(atob(value.data), (char) => char.charCodeAt(0))
      const objectUrl = URL.createObjectURL(new Blob([bytes], { type: value.mediaType }))
      url.value = objectUrl
      cleanup(() => URL.revokeObjectURL(objectUrl))
    },
    { immediate: true },
  )
  return url
}
