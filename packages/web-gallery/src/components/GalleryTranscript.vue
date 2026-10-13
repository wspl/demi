<script setup lang="ts">
import { computed } from 'vue'
import type { Block } from '@demicodes/protocol'
import { provideTranscript } from '@demicodes/web-ui/agent/edit-selection'
import { useMediaAddress } from '@demicodes/web-ui/agent/media-source'
import { transcriptPictures } from '@demicodes/web-ui/agent/transcript-pictures'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { storedRequests } from '@demicodes/web-ui/files/request-changes'
import { useGalleryTranscripts } from '../fixtures/transcripts'

/**
 * A block shown on its own, standing in the request `blocks` make, as a
 * message list would place it: its file pills open the request's changes
 * in the gallery's panel.
 */
const props = defineProps<{ blocks: Block[] }>()
const requests = computed(() => storedRequests(props.blocks))
const files = useMessageFiles()
const address = useMediaAddress()
provideTranscript({
  node: null,
  requests: () => requests.value,
  directory: () => null,
  pictures: () => transcriptPictures(props.blocks, { node: null, files, address }),
})
useGalleryTranscripts(() => ({ blocks: props.blocks, subagents: [] }))
</script>

<template>
  <slot />
</template>
