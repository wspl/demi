<script setup lang="ts">
import type { SentenceText } from '../ui/ui-text'
/** The outcome of the last action, under the group it belongs to. */
defineProps<{
  text: SentenceText
}>()
</script>

<template>
  <p role="status" class="select-none text-[12px] leading-4 text-fg-subtle">{{ text }}</p>
</template>
