<script setup lang="ts">
/** The outcome of the last action, under the group it belongs to. */
defineProps<{
  text: string
}>()
</script>

<template>
  <p role="status" class="select-none text-[12px] leading-4 text-fg-subtle">{{ text }}</p>
</template>
