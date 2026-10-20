<script setup lang="ts">
import { provide } from 'vue'
import { createOverlayFamily, overlayFamilyKey, overlayOwnerKey } from '../overlay/overlayFamily'
import { provideStacking } from '../overlay/stacking'
import type { OverlayLayerOf } from './context-menu'

/**
 * Where the page's context menu stands.
 *
 * - On the dialog or sheet it opened in, as a menu opened from a control in it
 *   does: a nested layer above it that Escape closes first and that leaves it open.
 * - On the page itself, given no layer, as any menu does.
 */
const props = defineProps<{
  layer: OverlayLayerOf | null
}>()

// Fixed for the component's life: the menu keys this component to each opening.
if (props.layer) {
  const { owner, z } = props.layer
  provide(overlayOwnerKey, owner)
  provide(overlayFamilyKey, createOverlayFamily())
  provideStacking(() => z)
}
</script>

<template>
  <slot />
</template>
