<script setup lang="ts">
import { COMPOSER_VEIL_FADE_PX, COMPOSER_VEIL_SCROLLBAR_GUTTER_PX } from './composer-clearance'
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
        var(--color-surface) 38%,
        color-mix(in srgb, var(--color-surface) 78%, transparent) 55%,
        color-mix(in srgb, var(--color-surface) 42%, transparent) 72%,
        color-mix(in srgb, var(--color-surface) 16%, transparent) 88%,
        transparent 100%)`,
    }"
  />
</template>
