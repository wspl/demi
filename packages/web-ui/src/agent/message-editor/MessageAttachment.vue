<script setup lang="ts">
import { computed } from 'vue'
import AttachmentCapsule from './AttachmentCapsule.vue'
import { useCapsuleOpening } from './capsule-opening'
import { useTransfers, type MessageCapsule } from './capsules'

/**
 * A file's capsule where it stands in a message: in the composer's editor,
 * through its node view, and in a sent message as it is drawn
 * (`SentMessage`). A click in the conversation opens the file.
 */
const props = defineProps<{
  /** The file as the message's node carries it. */
  saved: MessageCapsule | null
  /** It stands in a message being written, where a click opens nothing. */
  editable: boolean
  selected?: boolean
}>()
const transfers = useTransfers()
/**
 * The file this capsule stands for: as the composer knows it now, or as its
 * node carries it.
 */
const capsule = computed<MessageCapsule | null>(() => props.saved && (transfers?.current(props.saved.id) ?? props.saved))
/** How far the file is on its way, while the composer still carries it. */
const transfer = computed(() => {
  const id = capsule.value?.id
  return id && transfers ? transfers.transfer(id) : undefined
})
/**
 * A file on another device is not on the conversation's Host, so it opens
 * nothing.
 */
const { opens, open } = useCapsuleOpening(() => props.editable, () => capsule.value?.host ? null : capsule.value?.path)
</script>

<template>
  <AttachmentCapsule
    v-if="capsule"
    :capsule="capsule"
    :transfer="transfer"
    :selected="selected"
    :retryable="!!transfers?.retry"
    :class="opens ? 'cursor-pointer' : ''"
    data-drag-handle
    @click="open"
    @retry="transfers?.retry?.(capsule.id)"
  />
</template>
