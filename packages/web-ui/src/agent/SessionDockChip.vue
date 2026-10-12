<script setup lang="ts">
import StatusDot from '../ui/StatusDot.vue'

withDefaults(defineProps<{
  dot?: 'accent' | 'success' | 'muted'
  breathing?: boolean
  /** It cannot act now, as a Resume that waits for its device; its owner says why. */
  disabled?: boolean
}>(), {})
</script>

<template>
  <!-- The dock floats over the scrolling transcript, so a chip takes the opaque control surface. -->
  <span
    role="button"
    :aria-disabled="disabled || undefined"
    class="btn-solid inline-flex h-7 cursor-default select-none items-center gap-1.5 rounded-full px-2.5 text-chrome"
    :class="disabled ? 'text-fg-faint' : 'text-fg-body hover:text-fg-emphasis'"
  >
    <StatusDot v-if="dot" :tone="dot" :breathing="breathing" />
    <slot />
  </span>
</template>
