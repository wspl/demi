<script setup lang="ts">
import { computed } from 'vue'
import { CircleAlert, RotateCw } from '@lucide/vue'
import FileIcon from '../../files/FileIcon.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { useMediaUrl } from '../media-source'
import type { MessageCapsule } from './capsules'

/**
 * One file in a message's text: its picture or its kind's icon, then its
 * name, and the device a file on another device is on. Pointed at, it shows
 * the picture larger or a text file's opening lines. An upload in progress
 * shows how far it has come; a failed one offers Retry.
 *
 * The capsule stands on the line by its name, which keeps the baseline of the
 * text around it; the icon and Retry are centred in the pill. Hanging the
 * pill on its own middle instead would set the name a little below that line.
 *
 * It is a character of the message, so it is deleted as one, by the editor's
 * own keys; it carries no control of its own for that.
 */
const props = defineProps<{
  capsule: MessageCapsule
  /** The editor holds it as its selection. */
  selected?: boolean
}>()

const emit = defineEmits<{
  retry: []
}>()

const RING = 14
const STROKE = 2
const radius = (RING - STROKE) / 2
const circumference = 2 * Math.PI * radius

const loaded = useMediaUrl(() => typeof props.capsule.image === 'object' ? props.capsule.image : undefined)
const image = computed(() => typeof props.capsule.image === 'string' ? props.capsule.image : loaded.value)
const uploading = computed(() => props.capsule.upload?.phase === 'uploading' ? props.capsule.upload : null)
const failed = computed(() => props.capsule.upload?.phase === 'failed')
const caption = computed(() => {
  const { host, path, name } = props.capsule
  if (failed.value) {
    return `${name} did not upload. Retry, or delete it.`
  }
  if (uploading.value) {
    return `Uploading ${Math.round(uploading.value.progress * 100)}% · ${name}`
  }
  return host ? `${host} · ${path ?? name}` : path ?? name
})

/** What the capsule shows when pointed at; a file still on its way shows its caption alone. */
const preview = computed<
  { kind: 'picture'; src: string } | { kind: 'lines'; text: string } | { kind: 'caption' }
>(() => {
  if (uploading.value || failed.value) {
    return { kind: 'caption' }
  }
  if (image.value) {
    return { kind: 'picture', src: image.value }
  }
  return props.capsule.snippet ? { kind: 'lines', text: props.capsule.snippet } : { kind: 'caption' }
})
</script>

<template>
  <Tooltip
    tag="span"
    class="inline-flex max-w-full items-baseline align-baseline"
    :picture="preview.kind === 'picture'"
  >
    <span
      class="message-capsule mx-px inline-flex h-5 min-w-0 max-w-full select-none items-baseline gap-1 pl-[var(--capsule-inset)] pr-1.5 align-baseline text-[13px] font-normal not-italic leading-5 no-underline shadow-[var(--shadow-btn)]"
      :class="[
        failed ? 'bg-tint-danger text-on-danger' : 'bg-[var(--btn-bg)] text-fg-body',
        selected ? 'outline outline-2 outline-line-focus' : '',
      ]"
      :data-phase="capsule.upload?.phase"
      :aria-label="caption"
      :aria-busy="uploading ? true : undefined"
    >
      <svg
        v-if="uploading"
        role="progressbar"
        :aria-valuemin="0"
        :aria-valuemax="100"
        :aria-valuenow="Math.round(uploading.progress * 100)"
        :aria-label="capsule.name"
        class="mx-px shrink-0 -rotate-90 self-center"
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
          :stroke-dashoffset="circumference * (1 - uploading.progress)"
          class="text-fg-muted transition-[stroke-dashoffset] duration-75 ease-linear"
        />
      </svg>
      <CircleAlert
        v-else-if="failed"
        :size="14"
        class="mx-px shrink-0 self-center"
      />
      <img
        v-else-if="image"
        :src="image"
        alt=""
        class="message-capsule-thumb size-4 shrink-0 self-center object-cover"
      />
      <FileIcon
        v-else
        :name="capsule.name"
        :is-directory="false"
        :size="14"
        class="mx-px shrink-0 self-center"
      />
      <span
        class="min-w-0 truncate"
        :class="uploading ? 'text-fg-muted' : ''"
      >{{ capsule.name }}</span>
      <span
        v-if="capsule.host"
        class="shrink-0 text-fg-muted"
      >· {{ capsule.host }}</span>
      <button
        v-if="failed"
        type="button"
        class="-mr-1 flex size-4 shrink-0 self-center items-center justify-center rounded text-on-danger hover:bg-tint-danger-strong"
        aria-label="Retry"
        @click.stop="emit('retry')"
      >
        <RotateCw :size="11" />
      </button>
    </span>
    <template #overlay>
      <img
        v-if="preview.kind === 'picture'"
        :src="preview.src"
        :alt="capsule.name"
        class="block max-h-48 max-w-full rounded object-contain"
      />
      <pre
        v-else-if="preview.kind === 'lines'"
        class="max-h-40 overflow-hidden whitespace-pre-wrap break-all font-mono text-[11px] leading-4"
      >{{ preview.text }}</pre>
      <span
        v-else
        class="break-all"
      >{{ caption }}</span>
    </template>
  </Tooltip>
</template>
