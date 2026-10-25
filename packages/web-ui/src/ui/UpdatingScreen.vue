<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes } from '../files/format'
import ProgressBar from './ProgressBar.vue'
import { formatTimeLeft } from './time-left'
import { t } from './locales/text'

/** How far an update is, as `GET /api/update` says. */
export interface UpdateProgress {
  step: 'control' | 'conversations'
  conversationsDone: number
  conversations: number
  bytesDone: number
  bytes: number
  leftSeconds?: number | null
}

/**
 * The screen over the whole app while Demi restarts for an update an
 * administrator ran, or updates its data for a new release: what it updates
 * now, with the conversations and the bytes done of
 * how many, a bar of the share done, and the time left, as macOS Software
 * Update gives them. Nothing on it acts: the page loads itself once Demi
 * serves again.
 */
const props = defineProps<{
  /** How far the update is; null until Demi says, which shows the bar moving. */
  progress: UpdateProgress | null
  /** Demi restarts for an update, before it says how far its data is. */
  restarting?: boolean
}>()

const step = computed(() => {
  const progress = props.progress
  if (progress === null)
    return props.restarting ? t('updating.restarting') : ''
  return progress.step === 'control'
    ? t('updating.control')
    : t('updating.conversations', { done: progress.conversationsDone, count: progress.conversations })
})

const size = computed(() => props.progress === null
  ? ''
  : t('updating.bytes', { done: formatBytes(props.progress.bytesDone), all: formatBytes(props.progress.bytes) }))

const left = computed(() => props.progress === null ? '' : formatTimeLeft(props.progress.leftSeconds ?? null))
</script>

<template>
  <div
    class="fixed inset-0 z-[100] flex flex-col items-center justify-center gap-3 bg-surface-base px-6 text-center"
    role="status"
    aria-live="polite"
  >
    <h1 class="text-[15px] font-medium leading-5 text-fg-emphasis">{{ t('updating.title') }}</h1>
    <div class="w-full max-w-64">
      <ProgressBar v-if="progress" :value="progress.bytesDone" :max="progress.bytes" :label="t('updating.title')" />
      <div v-else class="relative h-1 w-full overflow-hidden rounded-full bg-overlay/8" role="progressbar" :aria-label="t('updating.title')">
        <div class="updating-moving absolute inset-y-0 w-1/3 rounded-full bg-accent-fill" />
      </div>
    </div>
    <div class="min-h-10 text-chrome">
      <p class="text-fg">{{ step }}</p>
      <p class="text-fg-muted tabular-nums">
        <template v-if="progress">{{ size }} · {{ left }}</template>
      </p>
    </div>
    <p class="w-full max-w-sm text-chrome text-fg-muted">{{ t('updating.body') }}</p>
  </div>
</template>

<style scoped>
.updating-moving {
  animation: updating-moving 1.2s ease-in-out infinite;
}

@keyframes updating-moving {
  from {
    left: -33%;
  }
  to {
    left: 100%;
  }
}

@media (prefers-reduced-motion: reduce) {
  .updating-moving {
    animation: none;
    left: 33%;
  }
}
</style>
