<script setup lang="ts">
import { computed } from 'vue'

/**
 * A status dot on the corner of an icon: a new file, a running conversation,
 * a directory that cannot be read. The parent is `relative`; the dot sits at
 * its top-right, cut out of what is painted behind it by a ring in that
 * color (`--fill-color`: the button, the row or the surface). No tone keeps the
 * dot in the layout at zero opacity, so a status can fade in and out.
 */
export type CornerDotTone = 'accent' | 'success' | 'warning' | 'danger' | 'muted'

const props = withDefaults(
  defineProps<{
    tone: CornerDotTone | null
    /** xs is 4px, sm 6px. */
    size?: 'xs' | 'sm'
    /** A live status: the dot breathes. */
    pulse?: boolean
    /** What the dot means, for assistive technology; decorative without it. */
    label?: string
  }>(),
  { size: 'sm', pulse: false, label: undefined },
)

const TONE: Record<CornerDotTone, string> = {
  accent: 'bg-on-accent',
  success: 'bg-on-success',
  warning: 'bg-on-warning',
  danger: 'bg-on-danger',
  muted: 'bg-fg-faint',
}

const classes = computed(() => [
  props.size === 'xs' ? 'size-1' : 'size-1.5',
  props.tone ? `${TONE[props.tone]} opacity-100` : 'opacity-0',
  props.pulse && props.tone ? 'animate-pulse' : '',
])
</script>

<template>
  <span
    class="pointer-events-none absolute -right-px -top-px rounded-full ring-1 ring-(--fill-color) transition-[background-color,opacity] duration-300"
    :class="classes"
    :role="label ? 'img' : undefined"
    :aria-label="label"
    :aria-hidden="label ? undefined : 'true'"
  />
</template>
