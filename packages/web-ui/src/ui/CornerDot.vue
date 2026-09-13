<script setup lang="ts">
import { computed } from 'vue'

/**
 * A status dot on the corner of an icon: a new file, a running conversation,
 * a directory that cannot be read. The parent is `relative`; the dot sits at
 * its top-right, cut out of the surface behind it by a ring in that
 * surface's color, so `ring` names where the icon sits. No tone keeps the
 * dot in the layout at zero opacity, so a status can fade in and out.
 */
export type CornerDotTone = 'accent' | 'success' | 'warning' | 'danger' | 'muted'
export type CornerDotRing = 'surface' | 'base' | 'float' | 'editor' | 'button'

const props = withDefaults(
  defineProps<{
    tone: CornerDotTone | null
    /** xs is 4px, sm 6px. */
    size?: 'xs' | 'sm'
    ring?: CornerDotRing
    /** A live status: the dot breathes. */
    pulse?: boolean
    /** What the dot means, for assistive technology; decorative without it. */
    label?: string
  }>(),
  { size: 'sm', ring: 'surface', pulse: false, label: undefined },
)

const TONE: Record<CornerDotTone, string> = {
  accent: 'bg-on-accent',
  success: 'bg-on-success',
  warning: 'bg-on-warning',
  danger: 'bg-on-danger',
  muted: 'bg-fg-faint',
}

const RING: Record<CornerDotRing, string> = {
  surface: 'ring-surface',
  base: 'ring-surface-base',
  float: 'ring-surface-float',
  editor: 'ring-surface-editor',
  button: 'ring-[var(--btn-bg)]',
}

const classes = computed(() => [
  props.size === 'xs' ? 'size-1' : 'size-1.5',
  RING[props.ring],
  props.tone ? `${TONE[props.tone]} opacity-100` : 'opacity-0',
  props.pulse && props.tone ? 'animate-pulse' : '',
])
</script>

<template>
  <span
    class="pointer-events-none absolute -right-px -top-px rounded-full ring-1 transition-[background-color,opacity] duration-300"
    :class="classes"
    :role="label ? 'img' : undefined"
    :aria-label="label"
    :aria-hidden="label ? undefined : 'true'"
  />
</template>
