<script setup lang="ts">
import { computed } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { useMessageFiles } from '../../markdown/message-files'
import AttachmentCapsule from './AttachmentCapsule.vue'
import { useTransfers, type MessageCapsule } from './capsules'

const props = defineProps(nodeViewProps)
const transfers = useTransfers()
const files = useMessageFiles()
/** The file this capsule stands for, as its node carries it. */
const capsule = computed<MessageCapsule | null>(() => props.node.attrs['capsule'] ?? null)
/** How far the file is on its way, while the composer still carries it. */
const transfer = computed(() => {
  const id = capsule.value?.id
  return id && transfers ? transfers.transfer(id) : undefined
})
/** In the conversation, a file on the conversation's Host opens in the File view. */
const opens = computed(() => {
  const path = capsule.value?.path
  return !props.editor.isEditable && !capsule.value?.host && path && files() ? path : null
})

function open(): void {
  if (opens.value) {
    files()?.open(opens.value)
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
    <AttachmentCapsule
      v-if="capsule"
      :capsule="capsule"
      :transfer="transfer"
      :selected="selected"
      :class="opens ? 'cursor-pointer' : ''"
      data-drag-handle
      @click="open"
      @retry="transfers?.retry(capsule.id)"
    />
  </NodeViewWrapper>
</template>
