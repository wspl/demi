<script setup lang="ts">
import { provide, ref } from 'vue'
import { overlayContainerKey, overlayInlineKey } from '@demicodes/web-ui/overlay/overlayContainer'

/**
 * A dialog shown as a plain panel, in flow at its own size: no scrim, no page-sized
 * well. The frame is still the overlay container, so the dialog never owns the page,
 * and menus opened inside it float over the frame's edge.
 */
const frame = ref<HTMLElement>()
provide(overlayContainerKey, frame)
provide(overlayInlineKey, true)
</script>

<template>
  <div ref="frame" class="relative isolate">
    <slot />
  </div>
</template>
