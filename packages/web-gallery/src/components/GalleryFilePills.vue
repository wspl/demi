<script setup lang="ts">
import { provideFilePills } from '@demicodes/web-ui/agent/edit-selection'

/**
 * Whether the transcript below shows its file pills, as the product's
 * conversation page decides from whether the conversation is in a project.
 */
const props = defineProps<{ shown: boolean }>()
provideFilePills(() => props.shown)
</script>

<template>
  <slot />
</template>
