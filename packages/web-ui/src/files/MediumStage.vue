<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
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
 * there.
 *
 * While the medium is on its way, the file's description or an image's bytes,
 * nothing shows for a moment, and then the loading pane, as a file view's, in
 * the middle, until the picture shows. A new version of a picture shown
 * replaces it in place with no loading pane.
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
  step: [direction: -1 | 1, slid: boolean]
  dismiss: []
}>()

const { src, gone, bytes } = useMediumSource(() => props.medium, () => props.open)
watch(bytes, (size) => emit('bytes', size), { immediate: true })
const failed = ref(false)
// Another medium, or a new version of the file, may show where the last could not.
watch([() => mediumKey(props.medium), src], () => {
  failed.value = false
})

/**
 * How long a medium on its way shows nothing before the loading pane, in
 * milliseconds: a picture that comes sooner, as one the web browser holds
 * does, shows with no spinner flashing before it.
 */
const LOADING_SHOWS_AFTER_MS = 300

/** The address whose picture loaded last, whatever opening it loaded in. */
const loaded = ref<string | null>(null)
/**
 * Whether this opening still waits for its picture: each opening, the same
 * medium's again included, waits until the address it shows has loaded, and
 * a new version that replaces the picture afterwards waits for nothing.
 */
const awaiting = ref(true)
watch(() => props.medium, () => {
  awaiting.value = true
})
watch([src, loaded, awaiting], () => {
  if (awaiting.value && src.value !== '' && src.value === loaded.value)
    awaiting.value = false
}, { immediate: true })
const waiting = computed(() =>
  !failed.value && !gone.value && (!src.value || (props.medium.kind === 'image' && awaiting.value)))
/** The loading pane shows: what shows took longer than a moment. */
const slow = ref(false)
watch(waiting, (now, _, onCleanup) => {
  slow.value = false
  if (!now)
    return
  const timer = setTimeout(() => {
    slow.value = true
  }, LOADING_SHOWS_AFTER_MS)
  onCleanup(() => clearTimeout(timer))
}, { immediate: true })

function onSize(width: number, height: number): void {
  loaded.value = src.value
  emit('size', width, height)
}
</script>

<template>
  <div class="relative h-full w-full">
    <RegionStatus v-if="failed || gone" class="h-full" status="failed" :label="medium.kind === 'image' ? t('view.imageFailed') : t('view.videoFailed')" />
    <ImagePreview
      v-else-if="medium.kind === 'image' && src"
      :key="mediumKey(medium)"
      :src="src"
      :name="medium.name"
      :flush="flush"
      :steps="steps"
      :from="from"
      :dismissible="dismissible"
      @size="onSize"
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
      @size="onSize"
      @failed="failed = true"
    />
    <RegionStatus
      v-if="slow"
      class="pointer-events-none absolute inset-0"
      status="loading"
      :label="shared('common.loading')"
    />
  </div>
</template>
