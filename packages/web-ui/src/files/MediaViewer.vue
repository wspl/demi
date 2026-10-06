<script setup lang="ts">
import { ref, watch } from 'vue'
import Dialog from '../ui/Dialog.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import ImagePreview from './ImagePreview.vue'
import MediaPreview from './MediaPreview.vue'
import type { MediaViewerState, ShownMedium } from './media-viewer'

/**
 * The image or video a viewer shows large over the dimmed page, as the File
 * view shows one: scaled down to fit and never enlarged, and its pixel size
 * beneath (`file-previews.md` § Media a tool returned). A click toggles an
 * image's actual size, and transparency shows over a checkerboard; a video
 * plays at once in the web browser's player, and closing the viewer stops
 * it. The page draws it beside the viewer it provides.
 */
const props = defineProps<{ viewer: MediaViewerState }>()

// The last medium stays while the viewer closes, so its leave shows the picture.
const medium = ref<ShownMedium | null>(null)
const size = ref<{ width: number; height: number } | null>(null)
const failed = ref(false)
watch(() => props.viewer.shown.value, (shown) => {
  if (!shown)
    return
  medium.value = shown
  size.value = null
  failed.value = false
})
</script>

<template>
  <Dialog
    :is-open="viewer.shown.value !== null"
    :overlay-store="appOverlayStore"
    size="full"
    :label="medium?.name"
    :scroll-content="false"
    close-over-content
    @close="viewer.close()"
  >
    <div v-if="medium" class="flex h-full min-h-0 flex-col">
      <div class="min-h-0 flex-1">
        <RegionStatus
          v-if="failed"
          class="h-full"
          failed
          :label="`Could not show this ${medium.kind}.`"
        />
        <ImagePreview
          v-else-if="medium.kind === 'image'"
          :src="medium.src"
          :name="medium.name"
          @size="(width, height) => size = { width, height }"
          @failed="failed = true"
        />
        <MediaPreview
          v-else
          :src="medium.src"
          kind="video"
          :name="medium.name"
          autoplay
          @size="(width, height) => size = { width, height }"
          @failed="failed = true"
        />
      </div>
      <div class="h-6 shrink-0 px-3 text-center text-[11px] tabular-nums text-fg-muted">
        {{ size ? `${size.width} × ${size.height}` : '' }}
      </div>
    </div>
  </Dialog>
</template>
