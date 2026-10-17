<script setup lang="ts">
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import MessageImage from './MessageImage.vue'

const props = defineProps(nodeViewProps)

// The wrapper breaks as the line around it does: tiptap's own `white-space:
// normal` would let the one-line composer wrap after it.
</script>

<template>
  <NodeViewWrapper
    as="span"
    style="white-space: inherit"
  >
    <MessageImage
      :target="String(props.node.attrs['src'] ?? '')"
      :alt="String(props.node.attrs['alt'] ?? '')"
      :linked="props.node.marks.some((mark) => mark.type.name === 'link')"
      :editable="props.editor.isEditable"
      :selected="selected"
    />
  </NodeViewWrapper>
</template>
