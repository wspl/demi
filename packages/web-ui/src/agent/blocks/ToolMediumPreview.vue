<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import type { ToolMediaSource } from '@demicodes/protocol'
import { useImageViewer } from '../../files/image-viewer'
import { dropSource } from '../../files/preview'
import { useMediaUrl } from '../media-source'

/**
 * One image or video a tool returned, at the preview's height, which it takes
 * before its bytes arrive so the transcript does not move when they load
 * (`file-previews.md` § Media a tool returned). An image is scaled down, never
 * enlarged or cropped, and a click opens it large; a video plays in the web
 * browser's player. A medium the page cannot show, because it did not load
 * or the web browser cannot decode it, says so in its place.
 */
const props = defineProps<{
  kind: 'image' | 'video'
  source: ToolMediaSource
  /** The call's title, which names the medium for the viewer and for a screen reader. */
  name: string
}>()

const src = useMediaUrl(() => props.source)
const viewer = useImageViewer()
const failed = ref(false)
const element = ref<HTMLImageElement | HTMLVideoElement | null>(null)
watch(src, () => {
  failed.value = false
})

onBeforeUnmount(() => {
  if (element.value)
    dropSource(element.value)
})
</script>

<template>
  <div class="flex h-preview max-w-full items-start">
    <div
      v-if="failed"
      class="flex h-full w-60 max-w-full items-center justify-center rounded-md border border-line-subtle px-3 text-center text-xs text-fg-muted"
    >
      Could not show this {{ kind }}.
    </div>
    <button
      v-else-if="kind === 'image'"
      type="button"
      class="max-w-full cursor-zoom-in rounded-md outline-none focus-visible:ring-2 focus-visible:ring-line-focus"
      :aria-label="`Open ${name} large`"
      @click="viewer.show(src, name)"
    >
      <img
        ref="element"
        :src="src"
        :alt="name"
        class="checkerboard block max-h-preview max-w-full rounded-md ring-1 ring-line"
        @error="failed = true"
      >
    </button>
    <video
      v-else
      ref="element"
      :src="src"
      :aria-label="name"
      controls
      preload="metadata"
      class="block h-full max-w-full rounded-md bg-black"
      @error="failed = true"
    />
  </div>
</template>
