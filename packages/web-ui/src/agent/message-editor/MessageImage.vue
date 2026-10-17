<script setup lang="ts">
import { computed } from 'vue'
import { hostImage } from '../../files/host-image'
import { transcriptRowOf, useMediaViewer } from '../../files/media-viewer'
import { useShowing } from '../../files/showing'
import { messageImage } from '../../markdown/filePath'
import { useMessageFiles } from '../../markdown/message-files'
import { useTranscript } from '../edit-selection'
import AttachmentCapsule from './AttachmentCapsule.vue'
import { useCapsuleOpening } from './capsule-opening'
import { imageCapsule } from './capsules'

/**
 * An image a message names, shown in the user's message as an attachment
 * capsule, never as a picture in its text (`file-previews.md` § Files named in
 * messages): the capsule of a file the user attached, in the composer and in
 * the sent message. The Markdown keeps the image. A Host image follows its
 * file, at the version of it the page keeps, so the capsule's thumbnail
 * changes when the file does. An image that loads from nowhere, and one whose
 * file is still being read, shows its alt text. A click does what an attached
 * image's capsule does; inside a link it follows the link.
 */
const props = defineProps<{
  /** The image's target, as the Markdown names it. */
  target: string
  alt: string
  /** It stands inside a link, which a click follows. */
  linked: boolean
  /** It stands in a message being written, where a click opens nothing. */
  editable: boolean
  selected?: boolean
}>()
const files = useMessageFiles()
const image = computed(() => messageImage(props.target, files()))
const path = computed(() => (image.value && 'path' in image.value ? image.value.path : null))
const file = useShowing(() => files()?.contents, () => path.value, (contents, at) => contents.showDescription(at))
const src = computed(() => {
  const shown = image.value
  if (!shown)
    return null
  if ('url' in shown)
    return shown.url
  const host = hostImage(file.entry.value, files()?.contents, shown.path)
  return host.state === 'shown' ? host.src : null
})
const capsule = computed(() => image.value && src.value !== null ? imageCapsule(image.value, src.value, props.alt) : null)
const { opens, open: openFile } = useCapsuleOpening(() => props.editable, () => props.linked ? null : path.value)
const viewer = useMediaViewer()
const transcript = useTranscript()
// A web image has no file to open, so it shows large in the media viewer,
// stepping through the transcript's pictures.
const viewable = computed(() => !props.editable && !props.linked && image.value !== null && 'url' in image.value && src.value !== null)

function open(event: MouseEvent): void {
  if (opens.value) {
    openFile()
  } else if (viewable.value && src.value !== null) {
    viewer.show({ kind: 'image', src: src.value, name: props.alt || 'image', size: null, row: transcriptRowOf(event.currentTarget instanceof Element ? event.currentTarget : null) }, transcript?.pictures)
  }
}
</script>

<template>
  <AttachmentCapsule
    v-if="capsule"
    :capsule="capsule"
    :selected="selected"
    :class="opens || viewable ? 'cursor-pointer' : ''"
    data-drag-handle
    @click="open"
  />
  <template v-else>{{ alt }}</template>
</template>
