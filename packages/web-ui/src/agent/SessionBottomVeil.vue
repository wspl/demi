<script setup lang="ts">
import { computed } from 'vue'
import { COMPOSER_VEIL_FADE, COMPOSER_VEIL_SCROLLBAR_GUTTER_PX } from './composer-clearance'

const props = defineProps<{
  /** The height of the dock's composer, 0 before the dock measures it. */
  composer: number
}>()

// The veil is opaque from the area's foot to the composer's top, the gaps
// between the composer's rows included, and fades evenly over the chips row
// above it, whether the row shows or not: there the scroll control floats.
// Before the dock measures its composer, the veil covers the area whole.
const height = computed(() =>
  props.composer > 0
    ? `calc(${props.composer}px + max(0.75rem, var(--safe-bottom)) + ${COMPOSER_VEIL_FADE})`
    : '100%',
)
</script>

<template>
  <div
    class="pointer-events-none absolute bottom-0 left-0"
    aria-hidden="true"
    :style="{
      right: `${COMPOSER_VEIL_SCROLLBAR_GUTTER_PX}px`,
      height,
      background: `linear-gradient(to top,
        var(--color-surface) 0,
        var(--color-surface) calc(100% - ${COMPOSER_VEIL_FADE}),
        transparent 100%)`,
    }"
  />
</template>
