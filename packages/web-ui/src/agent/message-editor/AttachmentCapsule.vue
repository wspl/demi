<script setup lang="ts">
import { computed, ref } from 'vue'
import { CircleAlert, RotateCw } from '@lucide/vue'
import FileIcon from '../../files/FileIcon.vue'
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import { declaredSize, type PixelSize } from '../../files/thumbnail'
import PicturePreview from '../../ui/PicturePreview.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { useMediaUrl } from '../media-source'
import type { MessageCapsule, MessageTransfer } from './capsules'

/**
 * One file in a message's text: its picture or its kind's icon, then its
 * name, and the device a file on another device is on. Pointed at, an image
 * or a video shows large in its preview card (`file-previews.md` § A
 * picture's preview on hover), and a text file its opening lines. An upload in progress
 * shows how far it has come; a failed one offers Retry.
 *
 * The capsule fits inside the line it stands in, at every text size: never
 * taller than the line, never reaching into the next one (`base.css`
 * § `.message-capsule-line`). Its icon, thumbnail, name and Retry are
 * centred in the pill.
 *
 * It is a character of the message, so it is deleted as one, by the editor's
 * own keys; it carries no control of its own for that.
 */
const props = defineProps<{
  capsule: MessageCapsule
  /** How far the file is on its way, while the composer still carries it. */
  transfer?: MessageTransfer
  /** The editor holds it as its selection, which it shows by a fill, never by the focus ring. */
  selected?: boolean
}>()

const emit = defineEmits<{
  retry: []
}>()

// The preview card is a second root, so the caller's class, drag handle and
// click go to the pill's line by hand.
defineOptions({ inheritAttrs: false })

const RING = 14
const STROKE = 2
const radius = (RING - STROKE) / 2
const circumference = 2 * Math.PI * radius

const loaded = useMediaUrl(() => typeof props.capsule.image === 'object' ? props.capsule.image : undefined)
const image = computed(() => typeof props.capsule.image === 'string' ? props.capsule.image : loaded.value)
const video = useMediaUrl(() => props.capsule.video)
const pill = ref<HTMLElement | null>(null)
/** The picture's size from its media block, known before its bytes arrive, else from the capsule's own thumbnail once loaded. */
const thumbSize = ref<PixelSize | null>(null)
const pictureSize = computed(() => {
  const source = props.capsule.image ?? props.capsule.video
  const declared = typeof source === 'object' && 'width' in source ? declaredSize(source) : null
  return declared ?? thumbSize.value
})
function onThumbLoad(event: Event): void {
  const thumb = event.target
  if (thumb instanceof HTMLImageElement && thumb.naturalWidth > 0)
    thumbSize.value = { width: thumb.naturalWidth, height: thumb.naturalHeight }
}
const uploading = computed(() => props.transfer?.phase === 'uploading' ? props.transfer : null)
const attaching = computed(() => props.transfer?.phase === 'attaching')
/** On its way: an upload, which tells how far it has come, or an attaching, which goes round. */
const busy = computed(() => !!uploading.value || attaching.value)
const failed = computed(() => props.transfer?.phase === 'failed')
const caption = computed(() => {
  const { host, path, name } = props.capsule
  if (failed.value) {
    return t('attachments.failed', { name })
  }
  if (uploading.value) {
    return t('attachments.uploading', { percent: Math.round(uploading.value.progress * 100), name })
  }
  if (attaching.value) {
    return t('attachments.attaching', { name })
  }
  return host ? `${host} · ${path ?? name}` : path ?? name
})

/** What the capsule shows when pointed at; a file still on its way shows its caption alone. */
const preview = computed<
  { kind: 'image' | 'video'; src: string } | { kind: 'lines'; text: string } | { kind: 'caption' }
>(() => {
  if (busy.value || failed.value) {
    return { kind: 'caption' }
  }
  if (image.value) {
    return { kind: 'image', src: image.value }
  }
  if (video.value) {
    return { kind: 'video', src: video.value }
  }
  return props.capsule.snippet ? { kind: 'lines', text: props.capsule.snippet } : { kind: 'caption' }
})
</script>

<template>
  <Tooltip
    v-bind="$attrs"
    tag="span"
    class="message-capsule-line inline-flex max-w-full"
    :disabled="preview.kind === 'image' || preview.kind === 'video'"
  >
    <span
      ref="pill"
      class="message-capsule mx-1 inline-flex h-full min-w-0 max-w-full select-none items-center gap-1 pl-[var(--capsule-inset)] pr-1.5 font-normal not-italic leading-none no-underline shadow-[var(--shadow-btn)]"
      :class="failed
        ? selected ? 'bg-tint-danger-strong text-on-danger' : 'bg-tint-danger text-on-danger'
        : selected ? 'bg-tint-accent text-fg-emphasis' : 'bg-btn text-fg-body'"
      :data-phase="transfer?.phase"
      :aria-label="caption"
      :aria-busy="busy ? true : undefined"
    >
      <svg
        v-if="busy"
        role="progressbar"
        :aria-valuemin="0"
        :aria-valuemax="100"
        :aria-valuenow="uploading ? Math.round(uploading.progress * 100) : undefined"
        :aria-label="capsule.name"
        class="mx-px shrink-0 -rotate-90"
        :class="attaching ? 'animate-spin' : ''"
        :width="RING"
        :height="RING"
        :viewBox="`0 0 ${RING} ${RING}`"
      >
        <circle
          :cx="RING / 2"
          :cy="RING / 2"
          :r="radius"
          fill="none"
          stroke="currentColor"
          :stroke-width="STROKE"
          class="text-overlay/10"
        />
        <circle
          :cx="RING / 2"
          :cy="RING / 2"
          :r="radius"
          fill="none"
          stroke="currentColor"
          :stroke-width="STROKE"
          stroke-linecap="round"
          :stroke-dasharray="circumference"
          :stroke-dashoffset="circumference * (1 - (uploading ? uploading.progress : 0.25))"
          class="text-fg-muted transition-[stroke-dashoffset] duration-75 ease-linear"
        />
      </svg>
      <CircleAlert
        v-else-if="failed"
        :size="14"
        class="mx-px shrink-0"
      />
      <img
        v-else-if="image"
        :src="image"
        alt=""
        class="message-capsule-thumb aspect-square shrink-0 object-cover"
        @load="onThumbLoad"
      />
      <FileIcon
        v-else
        :name="capsule.name"
        :is-directory="false"
        :size="14"
        class="mx-px shrink-0"
      />
      <span
        class="min-w-0 truncate"
        :class="busy ? 'text-fg-muted' : ''"
      >{{ capsule.name }}</span>
      <span
        v-if="capsule.host"
        class="shrink-0 text-fg-muted"
      >· {{ capsule.host }}</span>
      <button
        v-if="failed"
        type="button"
        class="-mr-1 flex size-4 shrink-0 items-center justify-center rounded text-on-danger hover:bg-tint-danger-strong"
        :aria-label="shared('common.retry')"
        @click.stop="emit('retry')"
      >
        <RotateCw :size="11" />
      </button>
    </span>
    <template #overlay>
      <pre
        v-if="preview.kind === 'lines'"
        class="max-h-40 overflow-hidden whitespace-pre-wrap font-mono wrap-anywhere text-[11px] leading-4"
      >{{ preview.text }}</pre>
      <span
        v-else
        class="wrap-anywhere"
      >{{ caption }}</span>
    </template>
  </Tooltip>
  <PicturePreview
    v-if="preview.kind === 'image' || preview.kind === 'video'"
    :trigger="pill"
    :kind="preview.kind"
    :src="preview.src"
    :size="pictureSize"
    :caption="capsule.name"
  />
</template>
