<script setup lang="ts">
withDefaults(defineProps<{
  dot?: 'accent' | 'success' | 'muted'
}>(), {})
</script>

<template>
  <span
    role="button"
    class="btn inline-flex h-7 cursor-default select-none items-center gap-1.5 rounded-full px-2.5 text-chrome text-fg-body hover:text-fg-emphasis"
  >
    <span
      v-if="dot"
      class="size-1.5 shrink-0 rounded-full"
      :class="dot === 'accent' ? 'bg-on-accent' : dot === 'success' ? 'bg-on-success' : 'bg-fg-subtle'"
    />
    <slot />
  </span>
</template>
