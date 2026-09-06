<script setup lang="ts">
import { provide, ref } from 'vue'
import { overlayContainerKey } from '@demicodes/web-ui/overlay/overlayContainer'

/**
 * A host for overlays pinned open. The well is a containing block, so the same floating
 * code that teleports to `body` in the product renders inside the well, clipped to it,
 * and never registers as the page's exclusive overlay.
 */
withDefaults(defineProps<{
  size?: 'md' | 'lg' | 'wide' | 'tall' | 'narrow'
}>(), {
  size: 'md',
})

const well = ref<HTMLElement>()
provide(overlayContainerKey, well)
</script>

<template>
  <div
    ref="well"
    class="gallery-overlay-well"
    :class="size === 'md' ? '' : `gallery-overlay-well-${size}`"
  >
    <slot />
  </div>
</template>
