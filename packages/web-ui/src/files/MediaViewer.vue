<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import { createReusableTemplate, useMediaQuery } from '@vueuse/core'
import Dialog from '../ui/Dialog.vue'
import FloatingWindow from '../ui/FloatingWindow.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import ImagePreview from './ImagePreview.vue'
import MediaPreview from './MediaPreview.vue'
import { formatCount } from '../ui/numbers'
import { formatZoom } from './image-zoom'
import { mediumKey, useMediumSource, type MediaViewerState, type ShownMedium } from './media-viewer'
import type { StepReach } from './swipe-step'
import type { PixelSize } from './thumbnail'

/**
 * The page's one media viewer (`file-previews.md` § The media viewer): a
 * small window over the page that the user moves, resizes and closes, while
 * the page stays usable. An image opens fitted to it, and zooms and moves as
 * in an image viewer (`ImagePreview`); a video plays at once in the web
 * browser's player, and closing the viewer stops it. The title bar names the
 * medium and gives its pixel size, and an image's zoom, *1280 × 720 · 150%*.
 * Opening another medium shows it in the same window, fitted.
 *
 * On a phone, the medium fills the screen instead, with its close control
 * (§ Media a tool returned, On a phone), and a swipe left or right steps to
 * the next or the previous picture (`ImagePreview`). The page draws it beside
 * the viewer it provides.
 */
const props = defineProps<{ viewer: MediaViewerState }>()

/** The medium itself, which the window and a phone's full screen both show. */
const [DefineMedium, ReuseMedium] = createReusableTemplate<{ flush: boolean; swipe: boolean }>()

/** A phone's width, the app's narrow width, where the viewer fills the screen (48rem, as `Dialog`'s). */
const phone = useMediaQuery('(max-width: 767.98px)')

// The last medium stays while the viewer closes, so its leave shows the picture.
const medium = shallowRef<ShownMedium | null>(null)
const size = ref<PixelSize | null>(null)
/** An image's zoom, which its preview reports; none for a video. */
const zoom = ref<number | null>(null)
const failed = ref(false)
const floating = ref<InstanceType<typeof FloatingWindow>>()
const open = computed(() => props.viewer.shown.value !== null)

/** Where the medium loads from: its address, or a Host image's file at the version the page keeps, followed while the viewer is open. */
const { src, gone } = useMediumSource(() => medium.value, () => open.value)

watch(() => props.viewer.shown.value, (shown, previous) => {
  if (!shown)
    return
  medium.value = shown
  size.value = shown.size ?? null
  zoom.value = null
  failed.value = false
  // A medium shown in place of another moves the focus into the window again, from the thumbnail that showed it.
  if (previous)
    floating.value?.activate()
})
// A new version of the file is another picture, which may show where the last could not.
watch(src, () => {
  failed.value = false
})

/** What the title bar tells of the medium: its place among the transcript's, its pixel size and an image's zoom, *3 / 12 · 1280 × 720 · 50%*. */
function details(withZoom: boolean): string {
  const place = props.viewer.position.value
  return [
    place ? `${formatCount(place.at)} / ${formatCount(place.of)}` : null,
    size.value ? `${size.value.width} × ${size.value.height}` : null,
    withZoom && size.value && zoom.value !== null ? formatZoom(zoom.value) : null,
  ].filter((part) => part !== null).join(' · ')
}

/** The arrow keys step to the medium before (left, up) or after (right, down); a video's player keeps its own. */
function onKeydown(event: KeyboardEvent): void {
  const direction = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 }[event.key]
  if (direction === undefined || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
    return
  if (event.target instanceof HTMLVideoElement)
    return
  event.preventDefault()
  props.viewer.step(direction === -1 ? -1 : 1)
}

/** The neighbours of the picture shown, which a swipe can step to; none when it stands in no transcript. */
const reach = computed<StepReach | undefined>(() => {
  const place = props.viewer.position.value
  return place ? { previous: place.at > 1, next: place.at < place.of } : undefined
})

/** The side the picture shown slid in from, when a swipe brought it. */
const arriving = ref<-1 | 0 | 1>(0)

watch(() => props.viewer.shown.value, () => {
  // Only the picture that mounts for a swipe's step slides in; one the page opens later does not.
  arriving.value = 0
}, { flush: 'post' })

function onSwipe(direction: -1 | 1, slid: boolean): void {
  arriving.value = slid ? direction : 0
  props.viewer.step(direction)
}

function onSize(width: number, height: number): void {
  size.value = { width, height }
  props.viewer.measured({ width, height })
}
</script>

<template>
  <DefineMedium v-slot="{ flush, swipe }">
    <template v-if="medium">
      <RegionStatus v-if="failed || gone" class="h-full" status="failed" :label="`Could not show this ${medium.kind}.`" />
      <ImagePreview
        v-else-if="medium.kind === 'image' && src"
        :key="mediumKey(medium)"
        :src="src"
        :name="medium.name"
        :flush="flush"
        :steps="swipe ? reach : undefined"
        :from="arriving"
        @size="onSize"
        @zoom="zoom = $event"
        @failed="failed = true"
        @step="onSwipe"
      />
      <MediaPreview
        v-else-if="medium.kind === 'video'"
        :key="mediumKey(medium)"
        :src="src"
        kind="video"
        :name="medium.name"
        autoplay
        :flush="flush"
        @size="onSize"
        @failed="failed = true"
      />
    </template>
  </DefineMedium>
  <Dialog
    v-if="phone"
    :is-open="open"
    :overlay-store="appOverlayStore"
    size="full"
    :label="medium?.name"
    :scroll-content="false"
    close-over-content
    @close="viewer.close()"
  >
    <div class="flex h-full min-h-0 flex-col">
      <div class="min-h-0 flex-1">
        <ReuseMedium :flush="false" :swipe="true" />
      </div>
      <div class="h-6 shrink-0 px-3 text-center text-caption tabular-nums text-fg-muted">
        {{ details(false) }}
      </div>
    </div>
  </Dialog>
  <FloatingWindow
    v-else-if="viewer.geometry.value"
    ref="floating"
    :open="open"
    :rect="viewer.geometry.value.rect"
    :label="medium?.name ?? ''"
    :overlay-store="appOverlayStore"
    @update:rect="viewer.place($event)"
    @close="viewer.close()"
    @keydown="onKeydown"
  >
    <template #title>{{ medium?.name }}</template>
    <template v-if="details(true)" #trailing>{{ details(true) }}</template>
    <ReuseMedium flush :swipe="false" />
  </FloatingWindow>
</template>
