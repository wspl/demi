<script setup lang="ts">
import { ref, watch } from 'vue'
import type {
  Base64ImageSource,
  DocumentSource,
  ImageSource,
  VideoSource,
} from '@demicodes/core'
import AttachmentTile from './AttachmentTile.vue'

/** Backend transcripts replace binary data with an authenticated blob reference. */
export interface BlobReferenceSource {
  type: 'ref'
  ref: string
  mediaType: string
  fileName?: string
}

const props = defineProps<{
  kind: 'image' | 'video' | 'document'
  source:
    | ImageSource
    | VideoSource
    | DocumentSource
    | BlobReferenceSource
    | Base64ImageSource
  name: string
}>()
const src = ref('')
watch(
  () => props.source,
  (source, _previous, cleanup) => {
    if ('type' in source && source.type === 'url') {
      src.value = source.url
      return
    }
    if ('type' in source && source.type === 'ref') {
      src.value = `/api/blobs/${encodeURIComponent(source.ref)}?type=${encodeURIComponent(source.mediaType)}`
      return
    }
    const data =
      typeof source.data === 'string'
        ? Uint8Array.from(atob(source.data), (char) => char.charCodeAt(0))
        : source.data
    const url = URL.createObjectURL(
      new Blob([data as BlobPart], { type: source.mediaType }),
    )
    src.value = url
    cleanup(() => URL.revokeObjectURL(url))
  },
  { immediate: true },
)
</script>

<template>
  <video
    v-if="kind === 'video'"
    :src="src"
    :aria-label="name"
    controls
    preload="metadata"
    class="max-h-64 max-w-full rounded-lg"
  />
  <a
    v-else
    :href="src"
    target="_blank"
    rel="noopener"
    :aria-label="name"
  >
    <AttachmentTile
      :name="name"
      :src="kind === 'image' ? src : undefined"
    />
  </a>
</template>
