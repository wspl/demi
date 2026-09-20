<script setup lang="ts">
import { computed } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { useMessageFiles } from '../../markdown/message-files'
import AttachmentCapsule from './AttachmentCapsule.vue'
import { useCapsules } from './capsules'

const props = defineProps(nodeViewProps)
const capsules = useCapsules()
const files = useMessageFiles()
const capsule = computed(() => capsules.capsule(String(props.node.attrs['id'])))
/** In the conversation, a file on the conversation's Host opens in the File view. */
const opens = computed(() => {
  const path = capsule.value?.path
  return !capsules.editable() && !capsule.value?.host && path && files() ? path : null
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
      :selected="selected"
      :class="opens ? 'cursor-pointer' : ''"
      data-drag-handle
      @click="open"
      @retry="capsules.retry(capsule.id)"
    />
  </NodeViewWrapper>
</template>
