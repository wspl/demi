<script setup lang="ts">
defineProps<{ muted: boolean }>()
</script>

<template>
  <div
    :inert="muted"
    :aria-disabled="muted || undefined"
    :data-edit-muted="muted || undefined"
    :class="muted && 'pointer-events-none select-none opacity-40'"
  >
    <slot />
  </div>
</template>
