<script setup lang="ts">
import { computed } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { useMessageFiles } from '../../markdown/message-files'
import MediumThumbnail from '../MediumThumbnail.vue'
import AttachmentCapsule from './AttachmentCapsule.vue'
import { useTransfers, type MessageCapsule } from './capsules'

const props = defineProps(nodeViewProps)
const transfers = useTransfers()
const files = useMessageFiles()
/** The file this capsule stands for: as the composer knows it now, or as its node carries it. */
const capsule = computed<MessageCapsule | null>(() => {
  const saved: MessageCapsule | null = props.node.attrs['capsule'] ?? null
  return saved && (transfers?.current(saved.id) ?? saved)
})
/** A sent message's image or video, which shows as its thumbnail where the capsule would stand. */
const thumbnail = computed(() => props.extension.options.thumbnails ? capsule.value?.medium ?? null : null)
/** How far the file is on its way, while the composer still carries it. */
const transfer = computed(() => {
  const id = capsule.value?.id
  return id && transfers ? transfers.transfer(id) : undefined
})
/** In the conversation, a file on the conversation's Host opens through the `file` intent, where a plugin opens it. */
const opens = computed(() => {
  const path = capsule.value?.path
  return !props.editor.isEditable && !capsule.value?.host && path && files()?.open ? path : null
})

function open(): void {
  if (opens.value) {
    files()?.open?.(opens.value)
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
    <MediumThumbnail
      v-if="thumbnail && capsule"
      class="message-thumbnail"
      :kind="thumbnail.kind"
      :source="thumbnail.source"
      :name="capsule.name"
    />
    <AttachmentCapsule
      v-else-if="capsule"
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
