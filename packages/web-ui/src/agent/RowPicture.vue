<script setup lang="ts">
import { t } from './locales/text'
import { t as files } from '../files/locales/text'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ImageOff } from '@lucide/vue'
import { transcriptRowOf, useMediaViewer } from '../files/media-viewer'
import { useTranscript } from './edit-selection'
import { dropSource } from '../files/preview'
import { declaredSize, loadedSize, type PixelSize } from '../files/thumbnail'
import PicturePreview from '../ui/PicturePreview.vue'
import Tooltip from '../ui/Tooltip.vue'
import { KEEPS_WINDOWS } from '../ui/window-stack'
import { useMediaUrl } from './media-source'
import type { ToolPicture } from './tool-media'
import { rowPictureBox } from './row-pictures'

/**
 * One image or video a tool returned, small in its row's line.
 *
 * - The box is 20 pixels tall, as wide as its proportions make it within 16 and
 *   48, cropped as a thumbnail is, with 4-pixel corners.
 * - The box comes from the size the picture's reference carries, known before
 *   its bytes arrive, so the row never moves when they load.
 * - A video shows its first frame with a play mark and never plays in place.
 * - Resting the pointer on the picture shows it large in a card. A click opens
 *   it in the viewer.
 * - One the page cannot show says so, pointed at, in its place.
 */
const props = defineProps<{ picture: ToolPicture }>()

const src = useMediaUrl(() => props.picture.source)
const viewer = useMediaViewer()
const transcript = useTranscript()
const failed = ref(false)
const element = ref<HTMLImageElement | HTMLVideoElement | null>(null)
/** The size its reference carries, known before its bytes arrive. */
const declared = computed(() => declaredSize(props.picture.source))
/** Its size once its bytes have loaded. */
const loaded = ref<PixelSize | null>(null)
const known = computed(() => declared.value ?? loaded.value)
const box = computed(() => rowPictureBox(props.picture))
watch(src, () => {
  failed.value = false
  loaded.value = null
})

function onLoaded(): void {
  if (element.value)
    loaded.value = loadedSize(element.value)
}

/**
 * Opens it in the page's viewer, which steps through the media of the
 * transcript it stands in.
 */
function openLarge(event: MouseEvent): void {
  const row = event.currentTarget instanceof Element ? transcriptRowOf(event.currentTarget) : undefined
  viewer.show({ kind: props.picture.kind, src: src.value, name: props.picture.name, size: known.value, row }, transcript?.pictures)
}

onBeforeUnmount(() => {
  if (element.value)
    dropSource(element.value)
})
</script>

<template>
  <Tooltip
    v-if="failed"
    :content="picture.kind === 'image' ? files('view.imageFailed') : files('view.videoFailed')"
    class="flex shrink-0 items-center"
  >
    <span
      class="flex items-center justify-center rounded border border-line-subtle text-fg-faint"
      :style="{ width: `${box.width}px`, height: `${box.height}px` }"
      :aria-label="picture.kind === 'image' ? files('view.imageFailed') : files('view.videoFailed')"
      role="img"
    >
      <ImageOff :size="12" />
    </span>
  </Tooltip>
  <PicturePreview
    v-else
    :kind="picture.kind"
    :src="src"
    :size="known"
    :caption="picture.name"
    class="flex shrink-0 items-center"
  >
    <button
      type="button"
      class="relative block rounded"
      :class="picture.kind === 'image' ? 'cursor-zoom-in' : 'cursor-pointer'"
      :aria-label="picture.kind === 'image' ? t('transcript.picture.open', { name: picture.name }) : t('transcript.picture.play', { name: picture.name })"
      v-bind="{ [KEEPS_WINDOWS]: '' }"
      @click.stop="openLarge"
    >
      <img
        v-if="picture.kind === 'image'"
        ref="element"
        :src="src"
        :alt="picture.name"
        class="block rounded object-cover object-top outline outline-1 -outline-offset-1 outline-line"
        :class="{ checkerboard: loaded }"
        :style="{ width: `${box.width}px`, height: `${box.height}px` }"
        @load="onLoaded"
        @error="failed = true"
      >
      <template v-else>
        <video
          ref="element"
          :src="src"
          :aria-label="picture.name"
          muted
          playsinline
          preload="metadata"
          class="block rounded bg-black object-cover object-top outline outline-1 -outline-offset-1 outline-line"
          :style="{ width: `${box.width}px`, height: `${box.height}px` }"
          @loadedmetadata="onLoaded"
          @error="failed = true"
        />
        <span class="media-play-mark" :style="{ width: '12px' }" aria-hidden="true" />
      </template>
    </button>
  </PicturePreview>
</template>
