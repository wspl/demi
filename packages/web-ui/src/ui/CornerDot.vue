<script setup lang="ts">
import StatusDot, { type StatusDotProps } from './StatusDot.vue'

/**
 * A status dot on the corner of an icon: a new file, a running conversation,
 * a directory that cannot be read. The parent is `relative`; the dot sits at
 * its top-right, cut out of what is painted behind it by a ring in that
 * color (`--fill-color`: the button, the row or the surface).
 */
const props = defineProps<StatusDotProps>()
</script>

<template>
  <StatusDot
    v-bind="props"
    class="pointer-events-none absolute -right-px -top-px ring-1 ring-(--fill-color)"
  />
</template>
