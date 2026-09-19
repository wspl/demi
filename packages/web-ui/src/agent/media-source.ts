import { ref, watch, type Ref } from 'vue'
import type { Base64ImageSource, DocumentSource, ImageSource, VideoSource } from '@demicodes/core'

/** Backend transcripts display media through authenticated blob references. */
export interface BlobReferenceSource {
  type: 'ref'
  ref: string
  mediaType: string
  fileName?: string
}

export type MediaSource = ImageSource | VideoSource | DocumentSource | BlobReferenceSource | Base64ImageSource

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
      if ('type' in value && value.type === 'url') {
        url.value = value.url
        return
      }
      if ('type' in value && value.type === 'ref') {
        url.value = `/api/blobs/${encodeURIComponent(value.ref)}?type=${encodeURIComponent(value.mediaType)}`
        return
      }
      // A copy owns a plain ArrayBuffer, which a Blob takes; the source's may be shared.
      const bytes =
        typeof value.data === 'string'
          ? Uint8Array.from(atob(value.data), (char) => char.charCodeAt(0))
          : value.data.slice()
      const objectUrl = URL.createObjectURL(new Blob([bytes], { type: value.mediaType }))
      url.value = objectUrl
      cleanup(() => URL.revokeObjectURL(objectUrl))
    },
    { immediate: true },
  )
  return url
}
