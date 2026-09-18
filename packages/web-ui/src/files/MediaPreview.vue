<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'

/** A video or audio file in the browser's own player. */
defineProps<{ src: string; kind: 'video' | 'audio'; name: string }>()
const emit = defineEmits<{ size: [width: number, height: number]; failed: [] }>()

const media = ref<HTMLMediaElement | null>(null)

function metadata(): void {
  const element = media.value
  if (element instanceof HTMLVideoElement)
    emit('size', element.videoWidth, element.videoHeight)
}

onBeforeUnmount(() => {
  // A removed player keeps downloading; the standard way to stop it is to
  // drop its source and load nothing.
  const element = media.value
  if (!element)
    return
  element.pause()
  element.removeAttribute('src')
  element.load()
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
