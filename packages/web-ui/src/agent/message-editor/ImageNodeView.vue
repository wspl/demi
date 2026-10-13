<script setup lang="ts">
import { computed } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { transcriptRowOf, useMediaViewer } from '../../files/media-viewer'
import { loadedSize } from '../../files/thumbnail'
import { messageImage } from '../../markdown/filePath'
import { useMessageFiles } from '../../markdown/message-files'
import { useTranscript } from '../edit-selection'

/**
 * An image a message names, loaded from the web or the conversation's Host;
 * its alt text when it loads from nowhere. In the conversation, a click shows
 * it large in the media viewer, stepping through the transcript's pictures.
 */
const props = defineProps(nodeViewProps)
const files = useMessageFiles()
const viewer = useMediaViewer()
const transcript = useTranscript()
const alt = computed(() => String(props.node.attrs['alt'] ?? ''))
const image = computed(() => messageImage(String(props.node.attrs['src'] ?? ''), files()))
const linked = computed(() => props.node.marks.some((mark) => mark.type.name === 'link'))

function open(event: MouseEvent): void {
  const shown = image.value
  const element = event.currentTarget
  // Editing, a click places the cursor; an image inside a link follows the link.
  if (!shown || !(element instanceof HTMLImageElement) || props.editor.isEditable || linked.value) {
    return
  }
  viewer.show({ kind: 'image', src: shown.src, name: alt.value || 'image', size: loadedSize(element), row: transcriptRowOf(element) }, transcript?.pictures)
}
// The wrapper breaks as the line around it does: tiptap's own `white-space:
// normal` would let the one-line composer wrap after it.
</script>

<template>
  <NodeViewWrapper
    as="span"
    style="white-space: inherit"
  >
    <img
      v-if="image"
      :src="image.src"
      :alt="alt"
      :title="node.attrs['title'] ?? undefined"
      class="message-image"
      :class="!editor.isEditable && !linked ? 'cursor-zoom-in' : ''"
      @click="open"
    />
    <template v-else>{{ alt }}</template>
  </NodeViewWrapper>
</template>
