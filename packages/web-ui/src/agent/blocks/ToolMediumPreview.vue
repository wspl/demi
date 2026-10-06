<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { ToolMediaSource } from '@demicodes/protocol'
import { useImageViewer } from '../../files/image-viewer'
import { dropSource } from '../../files/preview'
import { loadedSize, THUMBNAIL_HEIGHT, thumbnailBox, type PixelSize } from '../../files/thumbnail'
import { useMediaUrl } from '../media-source'

/**
 * One image or video a tool returned, which takes its box before its bytes
 * arrive so the transcript does not move when they load (`file-previews.md`
 * § Media a tool returned). An image is a thumbnail, cropped when its
 * proportions fall outside the thumbnail's, and a click opens it whole and
 * large; a video plays in the web browser's player, a player's height tall.
 * A medium the page cannot show, because it did not load or the web browser
 * cannot decode it, says so in its place.
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
/** The image's size once its bytes have loaded; the result does not carry it. */
const natural = ref<PixelSize | null>(null)
const box = computed(() => thumbnailBox(natural.value))
watch(src, () => {
  failed.value = false
  natural.value = null
})

function onImageLoad(): void {
  if (element.value instanceof HTMLImageElement)
    natural.value = loadedSize(element.value)
}

onBeforeUnmount(() => {
  if (element.value)
    dropSource(element.value)
})
</script>

<template>
  <div
    class="flex max-w-full items-start"
    :class="kind === 'video' ? 'h-video-player' : ''"
    :style="kind === 'image' ? { height: `${THUMBNAIL_HEIGHT}px` } : undefined"
  >
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
        class="block max-w-full rounded-md object-cover object-top ring-1 ring-line"
        :class="{ checkerboard: natural }"
        :style="{ width: `${box.width}px`, height: `${box.height}px` }"
        @load="onImageLoad"
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
