<script setup lang="ts">
import { ref, watch } from 'vue'
import { base64ToBytes } from '@demicodes/utils'
import AttachmentTile from './AttachmentTile.vue'
import type { DisplayedMediaSource } from './media-source'

const props = defineProps<{
  kind: 'image' | 'video' | 'document'
  source: DisplayedMediaSource
  name: string
  asAttachment?: boolean
  removable?: boolean
}>()
const emit = defineEmits<{ remove: [] }>()
const src = ref('')
watch(
  () => props.source,
  (source, _previous, cleanup) => {
    if ('type' in source && source.type === 'url') {
      src.value = source.url
      return
    }
    if ('ref' in source) {
      src.value = `/api/blobs/${encodeURIComponent(source.ref)}?type=${encodeURIComponent(source.mediaType)}`
      return
    }
    const data =
      typeof source.data === 'string'
        ? base64ToBytes(source.data)
        : source.data
    const url = URL.createObjectURL(
      new Blob([new Uint8Array(data)], { type: source.mediaType }),
    )
    src.value = url
    cleanup(() => URL.revokeObjectURL(url))
  },
  { immediate: true },
)
</script>

<template>
  <AttachmentTile
    v-if="asAttachment"
    :name="name"
    :src="kind === 'image' ? src : undefined"
    :removable="removable"
    @remove="emit('remove')"
  />
  <video
    v-else-if="kind === 'video'"
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
