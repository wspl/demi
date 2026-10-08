<script setup lang="ts">
import { computed } from 'vue'
import StatusDot, { type StatusDotProps } from './StatusDot.vue'

/**
 * A status dot on the corner of an icon: a new file, a running conversation,
 * a directory that cannot be read. The parent is `relative`; the dot sits at
 * its top-right, or its bottom-right where the icon's top-right is part of
 * its shape (a laptop's lid), cut out of what is painted behind it by a ring
 * in that color (`--fill-color`: the button, the row or the surface).
 */
const props = withDefaults(defineProps<StatusDotProps & {
  corner?: 'top-right' | 'bottom-right'
}>(), {
  size: 'sm',
  pulse: false,
  label: undefined,
  corner: 'top-right',
})

const dot = computed<StatusDotProps>(() => ({
  tone: props.tone,
  size: props.size,
  pulse: props.pulse,
  label: props.label,
}))
</script>

<template>
  <StatusDot
    v-bind="dot"
    class="pointer-events-none absolute -right-px ring-1 ring-(--fill-color)"
    :class="corner === 'top-right' ? '-top-px' : '-bottom-px'"
  />
</template>
