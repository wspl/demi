<script setup lang="ts">
import { computed } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { hostImage } from '../../files/host-image'
import { transcriptRowOf, useMediaViewer } from '../../files/media-viewer'
import { useShowing } from '../../files/showing'
import { loadedSize } from '../../files/thumbnail'
import { messageImage } from '../../markdown/filePath'
import { useMessageFiles } from '../../markdown/message-files'
import { useTranscript } from '../edit-selection'

/**
 * An image a message names, loaded from the web or the conversation's Host,
 * a Host image at the version of its file the page keeps, so it changes
 * when the file does; its alt text when it loads from nowhere, and while its
 * file is read. In the conversation, a click shows it large in the media
 * viewer, stepping through the transcript's pictures.
 */
const props = defineProps(nodeViewProps)
const files = useMessageFiles()
const viewer = useMediaViewer()
const transcript = useTranscript()
const alt = computed(() => String(props.node.attrs['alt'] ?? ''))
const image = computed(() => messageImage(String(props.node.attrs['src'] ?? ''), files()))
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
const linked = computed(() => props.node.marks.some((mark) => mark.type.name === 'link'))

function open(event: MouseEvent): void {
  const shown = image.value
  const contents = files()?.contents
  const element = event.currentTarget
  // Editing, a click places the cursor; an image inside a link follows the link.
  if (!shown || src.value === null || !(element instanceof HTMLImageElement) || props.editor.isEditable || linked.value) {
    return
  }
  const medium = { kind: 'image' as const, name: alt.value || 'image', size: loadedSize(element), row: transcriptRowOf(element) }
  // A Host image shows its file, at the version the page keeps, as the message does.
  viewer.show('path' in shown && contents ? { ...medium, file: { path: shown.path, contents } } : { ...medium, src: src.value }, transcript?.pictures)
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
      v-if="image && src !== null"
      :src="src"
      :alt="alt"
      :title="node.attrs['title'] ?? undefined"
      class="message-image"
      :class="!editor.isEditable && !linked ? 'cursor-zoom-in' : ''"
      @click="open"
    />
    <template v-else>{{ alt }}</template>
  </NodeViewWrapper>
</template>
