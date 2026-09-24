import { ref, watch, type Ref } from 'vue'
import type { DocumentSource, MediaSource as ContentMediaSource, ToolMediaSource } from '@demicodes/protocol'

/**
 * Where the bytes of an image, a video or a document are: an address, a blob
 * of the user's own, which the backend serves, or the bytes themselves.
 */
export type MediaSource = ContentMediaSource | DocumentSource | ToolMediaSource

/**
 * Where a media source loads from: its URL, the blob it references, or its
 * bytes as an object URL, which is released when the source changes or the
 * caller's scope ends. Empty while there is no source.
 */
export function useMediaUrl(source: () => MediaSource | undefined): Readonly<Ref<string>> {
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
        url.value = `/api/blobs/${encodeURIComponent(value.ref)}?type=${encodeURIComponent(value.mediaType)}`
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
