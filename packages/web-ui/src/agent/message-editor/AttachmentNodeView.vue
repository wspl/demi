<script setup lang="ts">
import { computed } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import AttachmentCapsule from './AttachmentCapsule.vue'
import { useCapsuleOpening } from './capsule-opening'
import { useTransfers, type MessageCapsule } from './capsules'

const props = defineProps(nodeViewProps)
const transfers = useTransfers()
/** The file this capsule stands for: as the composer knows it now, or as its node carries it. */
const capsule = computed<MessageCapsule | null>(() => {
  const saved: MessageCapsule | null = props.node.attrs['capsule'] ?? null
  return saved && (transfers?.current(saved.id) ?? saved)
})
/** How far the file is on its way, while the composer still carries it. */
const transfer = computed(() => {
  const id = capsule.value?.id
  return id && transfers ? transfers.transfer(id) : undefined
})
/** A file on another device is not on the conversation's Host, so it opens nothing. */
const { opens, open } = useCapsuleOpening(() => props.editor.isEditable, () => capsule.value?.host ? null : capsule.value?.path)

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
