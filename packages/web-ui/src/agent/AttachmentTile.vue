<script setup lang="ts">
import { computed } from 'vue'
import { X } from '@lucide/vue'
import { t } from '../infra/i18n'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import {
  attachmentProgress,
  type AttachmentPhase,
} from './message-input/attachments'

const props = defineProps<{
  name: string
  src?: string
  /** The opening of a text file; the tile shows it as a small page. */
  snippet?: string
  removable?: boolean
  phase?: AttachmentPhase
  progress?: number
}>()

const emit = defineEmits<{
  remove: []
}>()

const SIZE = 22
const STROKE = 2.5
const radius = (SIZE - STROKE) / 2
const circumference = 2 * Math.PI * radius
const percent = computed(() =>
  props.phase === 'uploading'
    ? attachmentProgress({
        phase: 'uploading',
        progress: props.progress,
      })
    : 0,
)
const dashOffset = computed(() => circumference * (1 - percent.value))
const percentLabel = computed(() => Math.round(percent.value * 100))
const extension = computed(() => props.name.split('.').pop()?.toUpperCase() ?? '')
</script>

<template>
  <span
    class="group/tile relative block size-12"
    :data-phase="phase"
    :aria-label="name"
    :aria-busy="phase === 'uploading' ? true : undefined"
  >
    <img
      v-if="src"
      :src="src"
      :alt="name"
      class="size-12 rounded-lg object-cover ring-1 ring-line"
      :class="phase === 'uploading' && 'opacity-40'"
    />
    <!-- A text file is a page: its first lines in type too small to read, and its kind in the corner. -->
    <span
      v-else-if="snippet"
      class="relative block size-12 overflow-hidden rounded-lg bg-surface ring-1 ring-line"
      :class="phase === 'uploading' && 'opacity-40'"
    >
      <span
        class="block h-full select-none whitespace-pre-wrap break-all px-1.5 pt-1.5 font-mono text-[5px] leading-[7px] text-fg-subtle"
        >{{ snippet }}</span
      >
      <span
        class="absolute bottom-0 right-0 rounded-tl-md bg-surface-raised px-1 text-[7px] font-medium leading-[10px] tracking-wide text-fg-muted ring-1 ring-line"
        >{{ extension }}</span
      >
    </span>
    <span
      v-else
      class="flex size-12 items-center justify-center rounded-lg bg-surface ring-1 ring-line"
      :class="phase === 'uploading' && 'opacity-40'"
    >
      <FileIcon
        :name="name"
        :is-directory="false"
        :size="28"
      />
    </span>
    <svg
      v-if="phase === 'uploading'"
      role="progressbar"
      :aria-valuemin="0"
      :aria-valuemax="100"
      :aria-valuenow="percentLabel"
      :aria-label="name"
      :data-progress="percent"
      class="absolute inset-0 m-auto -rotate-90"
      :width="SIZE"
      :height="SIZE"
      :viewBox="`0 0 ${SIZE} ${SIZE}`"
    >
      <circle
        :cx="SIZE / 2"
        :cy="SIZE / 2"
        :r="radius"
        fill="none"
        stroke="currentColor"
        :stroke-width="STROKE"
        class="text-overlay/8"
      />
      <circle
        :cx="SIZE / 2"
        :cy="SIZE / 2"
        :r="radius"
        fill="none"
        stroke="currentColor"
        :stroke-width="STROKE"
        stroke-linecap="round"
        :stroke-dasharray="circumference"
        :stroke-dashoffset="dashOffset"
        class="text-fg-muted transition-[stroke-dashoffset] duration-75 ease-linear"
      />
    </svg>
    <button
      v-if="removable"
      type="button"
      class="absolute -top-1.5 -right-1.5 flex size-4 items-center justify-center rounded-full bg-surface-raised text-fg-muted opacity-0 ring-1 ring-line transition-opacity duration-150 ease-out group-hover/tile:opacity-100 hover:text-fg-body focus-visible:opacity-100"
      :aria-label="t('common.close')"
      @click.stop="emit('remove')"
    >
      <X :size="ICON_PX.in12" />
    </button>
  </span>
</template>
