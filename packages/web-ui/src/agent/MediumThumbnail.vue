<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { MediaSource as ContentMediaSource, ToolMediaSource } from '@demicodes/protocol'
import { useMediaViewer } from '../files/media-viewer'
import { dropSource } from '../files/preview'
import { declaredSize, loadedSize, THUMBNAIL_HEIGHT, thumbnailBox, type PixelSize } from '../files/thumbnail'
import { useMediaUrl } from './media-source'

/**
 * One image or video of the conversation, as a thumbnail that takes its box
 * before its bytes arrive, from the size its reference carries, so the
 * transcript does not move when they load: one a tool returned
 * (`file-previews.md` § Media a tool returned), and one the user sent, where
 * its capsule stands (`product.md` § Attachments). It is cropped when its
 * proportions fall outside the thumbnail's; a video's is its first frame
 * with a play mark. A click opens it whole and large, where a video plays. A
 * medium the page cannot show, because it did not load or the web browser
 * cannot decode it, says so in its place.
 */
const props = defineProps<{
  kind: 'image' | 'video'
  source: ContentMediaSource | ToolMediaSource
  /** What names the medium for the viewer and for a screen reader: the call's title, or the file's name. */
  name: string
}>()

const src = useMediaUrl(() => props.source)
const viewer = useMediaViewer()
const failed = ref(false)
const element = ref<HTMLImageElement | HTMLVideoElement | null>(null)
/** The medium's size once its bytes have loaded. */
const loaded = ref<PixelSize | null>(null)
/** The medium's size as far as the page knows it: loaded, else what its reference declares. */
const known = computed(() => loaded.value ?? (props.source.type === 'ref' ? declaredSize(props.source) : null))
const box = computed(() => thumbnailBox(known.value, props.kind))
watch(src, () => {
  failed.value = false
  loaded.value = null
})

function onLoaded(): void {
  if (element.value)
    loaded.value = loadedSize(element.value)
}

/** Opens the medium in the page's viewer, whose window first opens over this thumbnail's pane. */
function openLarge(event: MouseEvent): void {
  const from = event.currentTarget instanceof Element ? event.currentTarget : null
  viewer.show({ kind: props.kind, src: src.value, name: props.name, size: known.value, from })
}

onBeforeUnmount(() => {
  if (element.value)
    dropSource(element.value)
})
</script>

<template>
  <div class="flex max-w-full items-start" :style="{ height: `${THUMBNAIL_HEIGHT}px` }">
    <div
      v-if="failed"
      class="flex h-full w-60 max-w-full items-center justify-center rounded-md border border-line-subtle px-3 text-center text-xs text-fg-muted"
    >
      Could not show this {{ kind }}.
    </div>
    <button
      v-else
      type="button"
      class="relative max-w-full rounded-md outline-none focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-line-focus"
      :class="kind === 'image' ? 'cursor-zoom-in' : 'cursor-pointer'"
      :aria-label="kind === 'image' ? `Open ${name} large` : `Play ${name}`"
      @click="openLarge"
    >
      <img
        v-if="kind === 'image'"
        ref="element"
        :src="src"
        :alt="name"
        class="block max-w-full rounded-md object-cover object-top outline outline-1 -outline-offset-1 outline-line"
        :class="{ checkerboard: loaded }"
        :style="{ width: `${box.width}px`, height: `${box.height}px` }"
        @load="onLoaded"
        @error="failed = true"
      >
      <template v-else>
        <video
          ref="element"
          :src="src"
          :aria-label="name"
          muted
          playsinline
          preload="metadata"
          class="block max-w-full rounded-md bg-black object-cover object-top outline outline-1 -outline-offset-1 outline-line"
          :style="{ width: `${box.width}px`, height: `${box.height}px` }"
          @loadedmetadata="onLoaded"
          @error="failed = true"
        />
        <span class="media-play-mark" aria-hidden="true" />
      </template>
    </button>
  </div>
</template>
