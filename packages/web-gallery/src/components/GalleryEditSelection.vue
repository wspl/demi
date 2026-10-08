<script setup lang="ts">
import { provideEditSelection, type EditSelectionHandler } from '@demicodes/web-ui/agent/edit-selection'

/**
 * What the file pills below open: `select`, or nothing at all, as with the
 * `changes` plugin off, when the pills are no controls.
 */
const props = defineProps<{ select?: EditSelectionHandler }>()
provideEditSelection(() => props.select)
</script>

<template>
  <slot />
</template>
