<script setup lang="ts">
import { COMPOSER_VEIL_FADE_PX, COMPOSER_VEIL_SCROLLBAR_GUTTER_PX } from './composer-clearance'

// The veil covers the dock whole, its rows and the gaps between them, however
// tall the dock grows, and fades evenly over the strip it reaches above it.
const fadeStart = `calc(100% - ${COMPOSER_VEIL_FADE_PX}px)`
</script>

<template>
  <div
    class="pointer-events-none absolute bottom-0 left-0"
    aria-hidden="true"
    :style="{
      right: `${COMPOSER_VEIL_SCROLLBAR_GUTTER_PX}px`,
      height: `calc(100% + ${COMPOSER_VEIL_FADE_PX}px)`,
      background: `linear-gradient(to top,
        var(--color-surface) 0,
        var(--color-surface) ${fadeStart},
        transparent 100%)`,
    }"
  />
</template>
