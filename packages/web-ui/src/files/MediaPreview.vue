<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import { dropSource } from './preview'

/**
 * A video or audio file in the web browser's own player, which plays at once
 * when `autoplay` says so, as a video opened large does. Leaving the page
 * stops it.
 */
defineProps<{ src: string; kind: 'video' | 'audio'; name: string; autoplay?: boolean }>()
const emit = defineEmits<{ size: [width: number, height: number]; failed: [] }>()

const media = ref<HTMLMediaElement | null>(null)

function metadata(): void {
  const element = media.value
  if (element instanceof HTMLVideoElement)
    emit('size', element.videoWidth, element.videoHeight)
}

onBeforeUnmount(() => {
  if (media.value)
    dropSource(media.value)
})
</script>

<template>
  <div class="flex h-full min-h-0 w-full items-center justify-center p-4">
    <video
      v-if="kind === 'video'"
      ref="media"
      :src="src"
      :aria-label="name"
      controls
      :autoplay="autoplay"
      preload="metadata"
      class="max-h-full max-w-full"
      @loadedmetadata="metadata"
      @error="emit('failed')"
    />
    <audio
      v-else
      ref="media"
      :src="src"
      :aria-label="name"
      controls
      preload="metadata"
      class="w-full max-w-md"
      @error="emit('failed')"
    />
  </div>
</template>
