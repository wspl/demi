<script setup lang="ts">
import AttachmentTile from './AttachmentTile.vue'
import { useMediaUrl, type MediaSource } from './media-source'

const props = defineProps<{
  kind: 'image' | 'video' | 'document'
  source: MediaSource
  name: string
}>()
const src = useMediaUrl(() => props.source)
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
