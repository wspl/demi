<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { fillScale } from './image-zoom'
import type { PixelSize } from './thumbnail'
import { dropSource } from './preview'
import VideoPlayer from './VideoPlayer.vue'

/**
 * A video in Demi's own player (`VideoPlayer`), which takes the whole space
 * as its stage, or an audio file in the web browser's, which plays at once
 * when `autoplay` says so, as a video opened large does. Leaving the page
 * stops it. A video shows as large as the space inside its margin lets it,
 * enlarged when it is smaller, as a picture opens (`ImagePreview`).
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

const player = ref<InstanceType<typeof VideoPlayer> | null>(null)
const audio = ref<HTMLAudioElement | null>(null)
/** The element that plays, the video's or the audio's. */
const media = computed<HTMLMediaElement | null>(() => (props.kind === 'video' ? player.value?.media ?? null : audio.value))
/** The space the player is shown in. */
const room = ref<HTMLElement | null>(null)
const { width: roomWidth, height: roomHeight } = useElementSize(room)
/** The margin around the video on each side, in pixels: the stage's `p-4`, none when `flush`. */
const margin = computed(() => (props.flush ? 0 : 16))
/** The video's pixel size, once its metadata came. */
const videoSize = shallowRef<PixelSize | null>(null)
watch(() => props.src, () => {
  videoSize.value = null
})
/**
 * The picture's box: the video as large as the space inside the margin lets
 * it, as a picture is fitted (`fillScale`). Until the size is known the
 * player is unseen.
 */
const videoBox = computed(() => {
  const size = videoSize.value
  const width = roomWidth.value - 2 * margin.value
  const height = roomHeight.value - 2 * margin.value
  if (!size || !size.width || !size.height || width <= 0 || height <= 0)
    return undefined
  const scale = fillScale(size, { width, height })
  return { width: `${size.width * scale}px`, height: `${size.height * scale}px` }
})
// After the render, so a new address the player was just given is the one
// paused, which keeps its autoplay from starting it.
watch([media, () => props.src, () => props.hidden], ([element, , hidden]) => {
  if (element && hidden)
    element.pause()
}, { flush: 'post' })

function metadata(video: HTMLVideoElement): void {
  videoSize.value = { width: video.videoWidth, height: video.videoHeight }
  emit('size', video.videoWidth, video.videoHeight)
}

onBeforeUnmount(() => {
  if (audio.value)
    dropSource(audio.value)
})
</script>

<template>
  <div ref="room" class="flex h-full min-h-0 w-full items-center justify-center" :class="{ 'p-4': !flush && kind === 'audio' }">
    <VideoPlayer
      v-if="kind === 'video'"
      ref="player"
      :src="src"
      :name="name"
      :autoplay="autoplay && !hidden"
      :picture="videoBox"
      class="size-full"
      :class="{ invisible: !videoBox }"
      @loadedmetadata="metadata"
      @error="emit('failed')"
    />
    <audio
      v-else
      ref="audio"
      :src="src"
      :aria-label="name"
      controls
      preload="metadata"
      class="w-full max-w-md"
      @error="emit('failed')"
    />
  </div>
</template>
