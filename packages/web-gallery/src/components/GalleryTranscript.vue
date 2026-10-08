<script setup lang="ts">
import { computed } from 'vue'
import type { Block } from '@demicodes/protocol'
import { provideTranscript } from '@demicodes/web-ui/agent/edit-selection'
import { transcriptRequests } from '@demicodes/web-ui/files/request-changes'
import { useGalleryTranscripts } from '../fixtures/transcripts'

/**
 * A block shown on its own, standing in the request `blocks` make, as a
 * message list would place it: its file pills open the request's changes
 * in the gallery's panel.
 */
const props = defineProps<{ blocks: Block[] }>()
const requests = computed(() => transcriptRequests(props.blocks))
provideTranscript({ node: null, requests: () => requests.value })
useGalleryTranscripts(() => ({ blocks: props.blocks, subagents: [] }))
</script>

<template>
  <slot />
</template>
