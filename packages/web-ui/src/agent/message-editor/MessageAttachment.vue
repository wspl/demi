<script setup lang="ts">
import { computed } from 'vue'
import { transcriptRowOf, useMediaViewer } from '../../files/media-viewer'
import { useTranscript } from '../edit-selection'
import { useMediaAddress } from '../media-source'
import { capsuleMedium } from '../transcript-pictures'
import AttachmentCapsule from './AttachmentCapsule.vue'
import { useCapsuleOpening } from './capsule-opening'
import { useMessageFiles } from '../../markdown/message-files'
import { useTransfers, type MessageCapsule } from './capsules'

/**
 * A file's capsule where it stands in a message: in the composer's editor,
 * through its node view, and in a sent message as it is drawn
 * (`SentMessage`). A click in the conversation opens the file: a picture or
 * a video in the page's viewer, at its place among the transcript's, and any
 * other file on the conversation's Host where a plugin opens it.
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
// A message being written opens nothing, so only a sent one asks for the
// page's viewer and for where media load from.
const viewer = props.editable ? null : useMediaViewer()
const address = props.editable ? null : useMediaAddress()
const transcript = useTranscript()
/** The picture or the video the file shows large; null for a file of another kind, or in a composer. */
const files = useMessageFiles()
const medium = computed(() => (capsule.value && address ? capsuleMedium(capsule.value, address, files()) : null))
/**
 * Any other file on another device is not on the conversation's Host, so it
 * opens nothing.
 */
const { opens, open: openFile } = useCapsuleOpening(
  () => props.editable || medium.value !== null,
  () => capsule.value?.host ? null : capsule.value?.path,
)

function open(event: MouseEvent): void {
  const shown = medium.value
  if (shown && viewer)
    viewer.show({ ...shown, row: transcriptRowOf(event.currentTarget instanceof Element ? event.currentTarget : null) }, transcript?.pictures)
  else
    openFile()
}
</script>

<template>
  <AttachmentCapsule
    v-if="capsule"
    :capsule="capsule"
    :transfer="transfer"
    :selected="selected"
    :retryable="!!transfers?.retry"
    :class="opens || medium ? 'cursor-pointer' : ''"
    data-drag-handle
    @click="open"
    @retry="transfers?.retry?.(capsule.id)"
  />
</template>
