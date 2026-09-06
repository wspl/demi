<script setup lang="ts">
/** A short status word: connected, beta, error. Never a control. */
withDefaults(defineProps<{
  tone?: 'neutral' | 'accent' | 'success' | 'warning' | 'danger'
}>(), {
  tone: 'neutral',
})
</script>

<template>
  <span
    class="inline-flex h-[18px] select-none items-center whitespace-nowrap rounded-full px-1.5 text-[11px] font-medium leading-4"
    :class="{
      'bg-hover text-fg-muted': tone === 'neutral',
      'bg-tint-accent text-on-accent': tone === 'accent',
      'bg-tint-success text-on-success': tone === 'success',
      'bg-tint-warning text-on-warning': tone === 'warning',
      'bg-tint-danger text-on-danger': tone === 'danger',
    }"
  >
    <slot />
  </span>
</template>
