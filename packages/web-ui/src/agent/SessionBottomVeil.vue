<script setup lang="ts">
import { computed } from 'vue'
import { COMPOSER_VEIL_FADE_PX, COMPOSER_VEIL_SCROLLBAR_GUTTER_PX, composerVeilOpacity } from './composer-clearance'

const props = defineProps<{
  /** The height of the dock's composer, 0 before the dock measures it. */
  composer: number
}>()

// The veil is opaque from the area's foot to the composer's top, the gaps
// between the composer's rows included, and fades over the chips row and the
// transcript above it, whether the row shows or not. Before the dock measures
// its composer, the veil covers the area whole.
const height = computed(() =>
  props.composer > 0
    ? `calc(${props.composer}px + max(0.75rem, var(--safe-bottom)) + ${COMPOSER_VEIL_FADE_PX}px)`
    : '100%',
)

/** The fade's color stops along its length, enough for the curve to read smooth. */
const FADE_STOPS = 24
const fade = Array.from({ length: FADE_STOPS + 1 }, (_, step) => {
  const t = step / FADE_STOPS
  const opacity = Math.round(composerVeilOpacity(t) * 1000) / 10
  return `color-mix(in srgb, var(--color-surface) ${opacity}%, transparent) calc(100% - ${(1 - t) * COMPOSER_VEIL_FADE_PX}px)`
}).join(', ')
</script>

<template>
  <div
    class="pointer-events-none absolute bottom-0 left-0"
    aria-hidden="true"
    :style="{
      right: `${COMPOSER_VEIL_SCROLLBAR_GUTTER_PX}px`,
      height,
      background: `linear-gradient(to top, var(--color-surface) 0, ${fade})`,
    }"
  />
</template>
