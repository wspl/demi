<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { fillScale } from './image-zoom'
import type { PixelSize } from './thumbnail'
import { dropSource } from './preview'

/**
 * A video or audio file in the web browser's own player, which plays at once
 * when `autoplay` says so, as a video opened large does. Leaving the page
 * stops it. A video shows as large as the space lets it, enlarged when it is
 * smaller, as a picture opens (`ImagePreview`).
 *
 * While `hidden` it never plays: a player that is playing pauses, and one
 * mounted or given another address meanwhile does not start. Shown again, it
 * waits for the user's Play.
 */
const props = defineProps<{
  src: string
  kind: 'video' | 'audio'
  name: string
  autoplay?: boolean
  /** It is out of sight, as in a tab of the work panel another tab covers. */
  hidden?: boolean
  /** Edge to edge, with no margin around the player, as a window that is sized to it shows it. */
  flush?: boolean
}>()
const emit = defineEmits<{ size: [width: number, height: number]; failed: [] }>()

const media = ref<HTMLMediaElement | null>(null)
/** The space the player is shown in. */
const room = ref<HTMLElement | null>(null)
const { width: roomWidth, height: roomHeight } = useElementSize(room)
/** The video's pixel size, once its metadata came. */
const videoSize = shallowRef<PixelSize | null>(null)
watch(() => props.src, () => {
  videoSize.value = null
})
/**
 * The player's box: the video as large as the space lets it, as a picture is
 * fitted (`fillScale`), and the player no larger than the video, so its
 * controls stay on it and what lies beside it stays the stage's. Until the
 * size is known the player keeps its own size.
 */
const videoBox = computed(() => {
  const size = videoSize.value
  if (!size || !size.width || !size.height || !roomWidth.value || !roomHeight.value)
    return undefined
  const scale = fillScale(size, { width: roomWidth.value, height: roomHeight.value })
  return { width: `${size.width * scale}px`, height: `${size.height * scale}px` }
})
// After the render, so a new address the player was just given is the one
// paused, which keeps its autoplay from starting it.
watch([media, () => props.src, () => props.hidden], ([element, , hidden]) => {
  if (element && hidden)
    element.pause()
}, { flush: 'post' })

function metadata(): void {
  const element = media.value
  if (element instanceof HTMLVideoElement) {
    videoSize.value = { width: element.videoWidth, height: element.videoHeight }
    emit('size', element.videoWidth, element.videoHeight)
  }
}

onBeforeUnmount(() => {
  if (media.value)
    dropSource(media.value)
})
</script>

<template>
  <div ref="room" class="flex h-full min-h-0 w-full items-center justify-center" :class="{ 'p-4': !flush }">
    <video
      v-if="kind === 'video'"
      ref="media"
      :src="src"
      :aria-label="name"
      controls
      :autoplay="autoplay && !hidden"
      preload="metadata"
      class="max-h-full max-w-full"
      :style="videoBox"
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
