<script setup lang="ts">
import { computed } from 'vue'

/**
 * A status dot: a device online or offline, a conversation running, a file
 * added. It stands in the flow wherever it is placed; `CornerDot` puts it on
 * an icon's corner. No tone keeps the dot in the layout at zero opacity, so a
 * status can fade in and out. `ring` draws it hollow, a settled mark that
 * holds the dot's place; `breathing` is the one look of a running thing, in
 * the sidebar row, the tabs, the status bar and the chips alike.
 */
export type StatusDotTone = 'accent' | 'success' | 'warning' | 'danger' | 'muted' | 'attention'

export interface StatusDotProps {
  tone: StatusDotTone | null
  /** xs is 4px, sm 6px. */
  size?: 'xs' | 'sm'
  /** A live status: the dot breathes (opacity and size, 2.4 s); still under reduced motion. */
  breathing?: boolean
  /** Hollow: a ring in the tone's color instead of a filled dot. */
  ring?: boolean
  /** What the dot means, for assistive technology; decorative without it. */
  label?: string
}

const props = withDefaults(defineProps<StatusDotProps>(), {
  size: 'sm',
  breathing: false,
  ring: false,
  label: undefined,
})

const TONE: Record<StatusDotTone, string> = {
  accent: 'bg-on-accent',
  success: 'bg-on-success',
  warning: 'bg-on-warning',
  danger: 'bg-on-danger',
  muted: 'bg-fg-faint',
  attention: 'bg-on-attention',
}

const RING_TONE: Record<StatusDotTone, string> = {
  accent: 'border-on-accent',
  success: 'border-on-success',
  warning: 'border-on-warning',
  danger: 'border-on-danger',
  muted: 'border-fg-faint/60',
  attention: 'border-on-attention',
}

const classes = computed(() => [
  props.size === 'xs' ? 'size-1' : 'size-1.5',
  props.tone ? (props.ring ? `border ${RING_TONE[props.tone]}` : TONE[props.tone]) : 'opacity-0',
  props.tone ? 'opacity-100' : '',
  props.breathing && props.tone ? 'status-dot-breathing' : '',
])
</script>

<template>
  <span
    class="shrink-0 rounded-full transition-[background-color,opacity] duration-300"
    :class="classes"
    :data-tone="tone ?? undefined"
    :role="label ? 'img' : undefined"
    :aria-label="label"
    :aria-hidden="label ? undefined : 'true'"
  />
</template>

<style scoped>
.status-dot-breathing {
  animation: status-dot-breathe 2.4s ease-in-out infinite;
}

@keyframes status-dot-breathe {
  0%,
  100% {
    opacity: 0.35;
    transform: scale(0.8);
  }
  50% {
    opacity: 1;
    transform: scale(1.15);
  }
}

@media (prefers-reduced-motion: reduce) {
  .status-dot-breathing {
    animation: none;
  }
}
</style>
