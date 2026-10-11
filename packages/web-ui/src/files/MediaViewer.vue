<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { createReusableTemplate, useMediaQuery } from '@vueuse/core'
import Dialog from '../ui/Dialog.vue'
import FloatingWindow from '../ui/FloatingWindow.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import ImagePreview from './ImagePreview.vue'
import MediaPreview from './MediaPreview.vue'
import type { MediaViewerState, ShownMedium } from './media-viewer'
import type { PixelSize } from './thumbnail'

/**
 * The page's one media viewer (`file-previews.md` § The media viewer): a
 * small window over the page that the user moves, resizes and closes, while
 * the page stays usable. An image fits it, scaled down and never enlarged,
 * over a checkerboard where it is transparent, and a click toggles its
 * actual size; a video plays at once in the web browser's player, and
 * closing the viewer stops it. The title bar names the medium and gives its
 * pixel size. Opening another medium shows it in the same window.
 *
 * On a phone, the medium fills the screen instead, with its close control
 * (§ Media a tool returned, On a phone). The page draws it beside the viewer
 * it provides.
 */
const props = defineProps<{ viewer: MediaViewerState }>()

/** The medium itself, which the window and a phone's full screen both show. */
const [DefineMedium, ReuseMedium] = createReusableTemplate<{ flush: boolean }>()

/** A phone's width, the app's narrow width, where the viewer fills the screen (48rem, as `Dialog`'s). */
const phone = useMediaQuery('(max-width: 767.98px)')

// The last medium stays while the viewer closes, so its leave shows the picture.
const medium = ref<ShownMedium | null>(null)
const size = ref<PixelSize | null>(null)
const failed = ref(false)
const floating = ref<InstanceType<typeof FloatingWindow>>()
const open = computed(() => props.viewer.shown.value !== null)

watch(() => props.viewer.shown.value, (shown, previous) => {
  if (!shown)
    return
  medium.value = shown
  size.value = shown.size ?? null
  failed.value = false
  // A medium shown in place of another moves the focus into the window again, from the thumbnail that showed it.
  if (previous)
    floating.value?.activate()
})

function onSize(width: number, height: number): void {
  size.value = { width, height }
  props.viewer.measured({ width, height })
}
</script>

<template>
  <DefineMedium v-slot="{ flush }">
    <template v-if="medium">
      <RegionStatus v-if="failed" class="h-full" status="failed" :label="`Could not show this ${medium.kind}.`" />
      <ImagePreview
        v-else-if="medium.kind === 'image'"
        :key="medium.src"
        :src="medium.src"
        :name="medium.name"
        :flush="flush"
        @size="onSize"
        @failed="failed = true"
      />
      <MediaPreview
        v-else
        :key="medium.src"
        :src="medium.src"
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
        <ReuseMedium :flush="false" />
      </div>
      <div class="h-6 shrink-0 px-3 text-center text-[11px] tabular-nums text-fg-muted">
        {{ size ? `${size.width} × ${size.height}` : '' }}
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
  >
    <template #title>{{ medium?.name }}</template>
    <template v-if="size" #trailing>{{ size.width }} × {{ size.height }}</template>
    <ReuseMedium flush />
  </FloatingWindow>
</template>
