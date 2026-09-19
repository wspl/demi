<script setup lang="ts">
import { computed } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { messageImage } from '../../markdown/filePath'
import { useMessageFiles } from '../../markdown/message-files'

/**
 * An image a message names, loaded from the web or the conversation's Host;
 * its alt text when it loads from nowhere. In the conversation, a click shows
 * it whole: a web image in a new tab, a Host image in the File view.
 */
const props = defineProps(nodeViewProps)
const files = useMessageFiles()
const alt = computed(() => String(props.node.attrs['alt'] ?? ''))
const image = computed(() => messageImage(String(props.node.attrs['src'] ?? ''), files()))
const linked = computed(() => props.node.marks.some((mark) => mark.type.name === 'link'))

function open(): void {
  const opens = image.value?.opens
  // Editing, a click places the cursor; an image inside a link follows the link.
  if (!opens || props.editor.isEditable || linked.value) {
    return
  }
  if ('web' in opens) {
    window.open(opens.web, '_blank', 'noopener,noreferrer')
  } else {
    files()?.open(opens.file)
  }
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
      :class="image.opens && !editor.isEditable ? 'cursor-pointer' : ''"
      @click="open"
    />
    <template v-else>{{ alt }}</template>
  </NodeViewWrapper>
</template>
