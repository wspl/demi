<script setup lang="ts">
import { ref, shallowRef, watch } from 'vue'
import RegionStatus from '../ui/RegionStatus.vue'
import ImagePreview from './ImagePreview.vue'
import MediaPreview from './MediaPreview.vue'
import { mediumKey, useMediumSource, type ShownMedium } from './media-viewer'
import type { StepReach } from './swipe-step'

/**
 * One medium shown large, as the viewer's window and the work panel's Media tab
 * show it: an image fitted to the space, which zooms and pans as in an image
 * viewer (`ImagePreview`), or a video that plays at once in the web browser's
 * player while `open`, and never while not. A Host image shows its file at
 * the version the page keeps while `open`, and says so when the file is not
 * there. An attachment that asks whether it opens the file it was uploaded
 * from (`origin`) shows nothing until it is answered, then that file or its
 * copy, so the copy never flashes before the file.
 *
 * Nothing shows before the medium has an address, as a Host file has none
 * while it is not `open`: a web browser fails a player given none, which
 * would leave the stage saying it could not show the medium.
 */
const props = defineProps<{
  medium: ShownMedium
  /** Whether the medium shows now, which a Host image's file is followed while. */
  open: boolean
  /** Edge to edge, with no margin around the medium. */
  flush: boolean
  /** The neighbours a swipe can step to; none when the picture is not swiped. */
  steps?: StepReach
  /** The side a swipe brought this picture from. */
  from?: -1 | 0 | 1
  /** A tap beside the picture closes it, where the picture covers the screen. */
  dismissible?: boolean
}>()
const emit = defineEmits<{
  size: [width: number, height: number]
  zoom: [scale: number]
  /** The size in bytes of what shows, null while it is not known. */
  bytes: [size: number | null]
  /** The medium as it shows: an attachment's file or its copy, once answered. */
  shown: [medium: ShownMedium]
  step: [direction: -1 | 1, slid: boolean]
  dismiss: []
}>()

/** The medium as it shows; null while an attachment's question is asked. */
const resolved = shallowRef<ShownMedium | null>(null)
watch(() => props.medium, (medium, _, onCleanup) => {
  const { origin } = medium
  if (!origin) {
    resolved.value = medium
    return
  }
  resolved.value = null
  let current = true
  onCleanup(() => {
    current = false
  })
  // A question that fails is an answer like any other but the file: the copy opens.
  void origin().catch(() => null).then((file) => {
    if (!current)
      return
    resolved.value = file
      ? { kind: medium.kind, name: medium.name, size: medium.size ?? null, ...(medium.row === undefined ? {} : { row: medium.row }), file }
      : medium
  })
}, { immediate: true })
watch(resolved, (medium) => {
  if (medium)
    emit('shown', medium)
})

const { src, gone, bytes } = useMediumSource(() => resolved.value, () => props.open)
watch(bytes, (size) => emit('bytes', size), { immediate: true })
const failed = ref(false)
// Another medium, or a new version of the file, may show where the last could not.
watch([() => mediumKey(props.medium), src], () => {
  failed.value = false
})
</script>

<template>
  <template v-if="resolved === null" />
  <RegionStatus v-else-if="failed || gone" class="h-full" status="failed" :label="`Could not show this ${medium.kind}.`" />
  <ImagePreview
    v-else-if="medium.kind === 'image' && src"
    :key="mediumKey(medium)"
    :src="src"
    :name="medium.name"
    :flush="flush"
    :steps="steps"
    :from="from"
    :dismissible="dismissible"
    @size="(width, height) => emit('size', width, height)"
    @zoom="emit('zoom', $event)"
    @failed="failed = true"
    @step="(direction, slid) => emit('step', direction, slid)"
    @dismiss="emit('dismiss')"
  />
  <MediaPreview
    v-else-if="medium.kind === 'video' && src"
    :key="mediumKey(medium)"
    :src="src"
    kind="video"
    :name="medium.name"
    autoplay
    :hidden="!open"
    :flush="flush"
    @size="(width, height) => emit('size', width, height)"
    @failed="failed = true"
  />
</template>
