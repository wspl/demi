<script setup lang="ts">
import { computed } from 'vue'

/**
 * A status dot: a device online or offline, a conversation running, a file
 * added. It stands in the flow wherever it is placed; `CornerDot` puts it on
 * an icon's corner. No tone keeps the dot in the layout at zero opacity, so a
 * status can fade in and out.
 */
export type StatusDotTone = 'accent' | 'success' | 'warning' | 'danger' | 'muted'

export interface StatusDotProps {
  tone: StatusDotTone | null
  /** xs is 4px, sm 6px. */
  size?: 'xs' | 'sm'
  /** A live status: the dot breathes. */
  pulse?: boolean
  /** What the dot means, for assistive technology; decorative without it. */
  label?: string
}

const props = withDefaults(defineProps<StatusDotProps>(), {
  size: 'sm',
  pulse: false,
  label: undefined,
})

const TONE: Record<StatusDotTone, string> = {
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
    class="shrink-0 rounded-full transition-[background-color,opacity] duration-300"
    :class="classes"
    :role="label ? 'img' : undefined"
    :aria-label="label"
    :aria-hidden="label ? undefined : 'true'"
  />
</template>
