<script setup lang="ts">
import { computed, nextTick, ref, watch, watchEffect } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { ChevronLeft, ChevronRight, Download } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { formatCount } from '../ui/numbers'
import { usePhone } from '../ui/phone'
import { downloadUrl } from './download'
import MediumStage from './MediumStage.vue'
import { mediumKey, useMediaViewer, type MediumPlace } from './media-viewer'
import FileIcon from './FileIcon.vue'
import { formatBytes } from './format'
import { baseName } from '@demicodes/utils'
import { ICON_PX } from '../ui/icon-metrics'
import type { StepReach } from './swipe-step'
import type { PixelSize } from './thumbnail'
import { t } from './locales/text'

/**
 * The work panel's Media tab: the medium the conversation's viewer shows, one
 * of the series of the conversation's pictures and videos, beside the
 * conversation.
 *
 * - The bar starts with its file's icon and name, as the Files tab shows a
 *   file; it ends with its resolution, its size, Download, then Previous,
 *   its place in the series, *3 / 12*, and Next.
 * - The Left and Right Arrow keys step through the series, and on a phone a
 *   swipe does; a video's player keeps its own keys.
 * - A picture zooms and pans as in the viewer's window (`ImagePreview`); a
 *   video plays in the web browser's player, and pauses while another tab
 *   shows.
 */
const props = defineProps<{
  /** Whether the tab is the panel's selection. */
  shown: boolean
  /** Where the medium it showed stands in the conversation's series, which it shows again after a reload. */
  place?: MediumPlace
}>()
const emit = defineEmits<{
  /** The medium shown moved to another place of the series. */
  place: [place: MediumPlace]
}>()

const viewer = useMediaViewer()
const phone = usePhone()
const root = ref<HTMLElement | null>(null)

const medium = computed(() => viewer.shown.value)

// A tab the page restored shows the medium it showed again, once the
// conversation's transcript offers the series that holds it.
watchEffect(() => {
  if (!viewer.shown.value && props.place)
    viewer.restore(props.place)
})
// The tab keeps where the medium shown stands, for the next reload.
watch(medium, (shown) => {
  if (!shown)
    return
  const place: MediumPlace = { key: mediumKey(shown), ...(shown.row === undefined ? {} : { row: shown.row }) }
  if (place.key !== props.place?.key || place.row !== props.place?.row)
    emit('place', place)
}, { immediate: true })
/** The medium's pixel size, from its reference until it loads, and its size in bytes, as the stage reports them. */
const size = ref<PixelSize | null>(null)
const bytes = ref<number | null>(null)
watch(medium, (shown) => {
  size.value = shown?.size ?? null
}, { immediate: true })
const resolution = computed(() => (size.value ? `${size.value.width} × ${size.value.height}` : null))
/** What the icon is told: a Host image's file, whose name its caption need not carry, or the medium's name. */
const iconName = computed(() => {
  const shown = medium.value
  return shown?.file ? baseName(shown.file.path) : shown?.name ?? ''
})

const bar = ref<HTMLElement | null>(null)
const nameBox = ref<HTMLElement | null>(null)
/** What of the end's text the bar keeps: 2 the resolution and the size, 1 the resolution, 0 neither. */
const kept = ref(2)
/**
 * Keeps as much of the end's text as leaves the name whole: the size goes
 * first, then the resolution, and only with both gone is the name cut. It
 * runs before the bar paints, from its resize and from a change of what it
 * says, so nothing shows the step between.
 */
async function fit(): Promise<void> {
  for (let next = 2; next >= 0; next--) {
    kept.value = next
    await nextTick()
    const name = nameBox.value?.firstElementChild
    if (!name || name.scrollWidth <= name.clientWidth)
      return
  }
}
useResizeObserver(bar, () => void fit())
watch([() => medium.value?.name, resolution, bytes], () => void fit())
const place = computed(() => viewer.position.value)
const reach = computed<StepReach>(() => {
  const at = place.value
  return { previous: at !== null && at.at > 1, next: at !== null && at.at < at.of }
})

/** The side the picture shown slid in from, when a swipe brought it. */
const arriving = ref<-1 | 0 | 1>(0)
function onSwipe(direction: -1 | 1, slid: boolean): void {
  arriving.value = slid ? direction : 0
  viewer.step(direction)
}

// A medium opened from the conversation takes the focus, the tab's first
// included, so the arrow keys step from it; only a swipe's step slides in.
watch(() => viewer.shown.value, () => {
  arriving.value = 0
}, { flush: 'post' })
watch([() => viewer.shown.value, () => props.shown], ([shown, selected]) => {
  if (shown && selected)
    void nextTick(() => root.value?.focus({ preventScroll: true }))
}, { immediate: true })

/** The arrow keys step to the medium before (left) or after (right); a video's player keeps its own. */
function onKeydown(event: KeyboardEvent): void {
  const direction = { ArrowLeft: -1, ArrowRight: 1 }[event.key]
  if (direction === undefined || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
    return
  if (event.target instanceof HTMLVideoElement)
    return
  event.preventDefault()
  viewer.step(direction === -1 ? -1 : 1)
}

/** Downloads the medium as it shows: a Host file's version, or its address. */
function download(): void {
  const shown = medium.value
  if (!shown)
    return
  if (shown.file)
    downloadUrl(shown.file.contents.url(shown.file.path, { download: true }))
  else
    downloadUrl(shown.src, shown.name)
}
</script>

<template>
  <div ref="root" tabindex="-1" class="flex h-full min-h-0 flex-col outline-none" data-media-view @keydown="onKeydown">
    <!-- The file's icon and name, as the Files tab shows a file, at the start;
         its resolution, its size, Download, then Previous, the place and Next
         at the end. In a narrow bar the size hides first, then the
         resolution, and only then is the name cut (`fit`). -->
    <div
      v-if="medium"
      ref="bar"
      class="flex h-9 shrink-0 select-none items-center gap-1 border-b border-line px-1.5 text-detail text-fg-muted phone:h-row"
    >
      <FileIcon class="ml-1.5 shrink-0" :name="iconName" :is-directory="false" :size="ICON_PX.in28" />
      <span ref="nameBox" class="flex min-w-0 flex-1">
        <TruncatedText class="min-w-0 text-fg" :text="medium.name" />
      </span>
      <span v-if="resolution && kept >= 1" class="ml-2 shrink-0 tabular-nums">{{ resolution }}</span>
      <span v-if="bytes !== null && kept >= 2" class="ml-2 shrink-0 tabular-nums">{{ formatBytes(bytes) }}</span>
      <Tooltip :content="t('actions.download')">
        <IconButton :icon="Download" variant="ghost" size="sm" :aria-label="t('actions.download')" @click="download" />
      </Tooltip>
      <Tooltip :content="t('media.previous')">
        <IconButton :icon="ChevronLeft" variant="ghost" size="sm" :aria-label="t('media.previous')" :disabled="!reach.previous" @click="viewer.step(-1)" />
      </Tooltip>
      <span v-if="place" class="shrink-0 tabular-nums">{{ `${formatCount(place.at)} / ${formatCount(place.of)}` }}</span>
      <Tooltip :content="t('media.next')">
        <IconButton :icon="ChevronRight" variant="ghost" size="sm" :aria-label="t('media.next')" :disabled="!reach.next" @click="viewer.step(1)" />
      </Tooltip>
    </div>
    <div class="min-h-0 flex-1">
      <MediumStage
        v-if="medium"
        :medium="medium"
        :open="shown"
        flush
        :steps="phone ? reach : undefined"
        :from="arriving"
        @size="(width, height) => (size = { width, height })"
        @bytes="bytes = $event"
        @step="onSwipe"
      />
      <RegionStatus v-else class="h-full" status="note" :label="t('media.none')" />
    </div>
  </div>
</template>
