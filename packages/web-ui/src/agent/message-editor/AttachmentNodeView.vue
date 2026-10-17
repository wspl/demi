<script setup lang="ts">
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import MessageAttachment from './MessageAttachment.vue'

const props = defineProps(nodeViewProps)

// The wrapper breaks as the line around it does: tiptap's own `white-space:
// normal` would let the one-line composer wrap after it.
</script>

<template>
  <NodeViewWrapper
    as="span"
    style="white-space: inherit"
  >
    <MessageAttachment
      :saved="props.node.attrs['capsule'] ?? null"
      :editable="props.editor.isEditable"
      :selected="selected"
    />
  </NodeViewWrapper>
</template>
