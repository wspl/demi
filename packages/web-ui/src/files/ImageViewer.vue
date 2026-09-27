<script setup lang="ts">
import { ref, watch } from 'vue'
import Dialog from '../ui/Dialog.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import ImagePreview from './ImagePreview.vue'
import type { ImageViewerState, ShownImage } from './image-viewer'

/**
 * The image a viewer shows large over the dimmed page, as the File view
 * shows one: scaled down to fit and never enlarged, a click toggles actual
 * size, transparency over a checkerboard, and its pixel size beneath
 * (`file-previews.md` § Media a tool returned). The page draws it beside
 * the viewer it provides.
 */
const props = defineProps<{ viewer: ImageViewerState }>()

// The last image stays while the viewer closes, so its leave shows the picture.
const image = ref<ShownImage | null>(null)
const size = ref<{ width: number; height: number } | null>(null)
const failed = ref(false)
watch(() => props.viewer.shown.value, (shown) => {
  if (!shown)
    return
  image.value = shown
  size.value = null
  failed.value = false
})
</script>

<template>
  <Dialog
    :is-open="viewer.shown.value !== null"
    :overlay-store="appOverlayStore"
    size="full"
    :label="image?.name"
    :scroll-content="false"
    @close="viewer.close()"
  >
    <div v-if="image" class="flex h-full min-h-0 flex-col">
      <div class="min-h-0 flex-1">
        <RegionStatus
          v-if="failed"
          class="h-full"
          failed
          label="Could not show this image."
        />
        <ImagePreview
          v-else
          :src="image.src"
          :name="image.name"
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
