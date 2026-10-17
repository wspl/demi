<script setup lang="ts">
import CornerDot from './CornerDot.vue'
import type { MenuIndicator } from './MenuItem.vue'

/**
 * A chosen item's icon on a closed dropdown's face, as its menu row draws it
 * (MenuItem): the icon, from the default slot, in the row's 16 px box, and
 * the row's status dot on its corner.
 */
defineProps<{
  indicator?: MenuIndicator
  indicatorLabel?: string
}>()
</script>

<template>
  <span class="relative flex size-4 shrink-0 items-center justify-center">
    <slot />
    <CornerDot v-if="indicator" :tone="indicator" :label="indicatorLabel" />
  </span>
</template>
