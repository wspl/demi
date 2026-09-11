<script setup lang="ts">
import { computed } from 'vue'
import { RotateCcw, X } from '@lucide/vue'
import { t } from '../infra/i18n'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import {
  attachmentProgress,
  type AttachmentDestination,
  type AttachmentPhase,
} from './message-input/attachments'

const props = defineProps<{
  name: string
  src?: string
  removable?: boolean
  destination?: AttachmentDestination
  phase?: AttachmentPhase
  progress?: number
}>()

const emit = defineEmits<{
  remove: []
  retry: []
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
</script>

<template>
  <span
    class="group/tile relative block size-12"
    :data-phase="phase"
    :aria-label="name"
    :aria-busy="phase === 'uploading' ? true : undefined"
  >
    <!-- A failed tile shows only its Retry: the preview would show through the control. -->
    <span
      v-if="phase === 'failed'"
      class="block size-12 rounded-lg bg-surface ring-1 ring-line"
    />
    <img
      v-else-if="src"
      :src="src"
      :alt="name"
      class="size-12 rounded-lg object-cover ring-1 ring-line"
      :class="phase === 'uploading' && 'opacity-40'"
    />
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
      v-if="phase === 'failed'"
      type="button"
      class="absolute inset-0 m-auto flex size-7 items-center justify-center rounded-full text-on-danger"
      :aria-label="`Retry upload ${name}`"
      @click.stop="emit('retry')"
    >
      <RotateCcw :size="16" />
    </button>
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
