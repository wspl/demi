<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { formatBytes } from '../files/format'
import ProgressBar from './ProgressBar.vue'
import UpdateSteps from './UpdateSteps.vue'
import { formatSpan } from './dates'
import { formatTimeLeft } from './time-left'
import { overallDone, type StepLine, type StepProgress, type StepState } from './update-steps'
import { t } from './locales/text'

/** The steps of an update from its restart on, in order. */
export type RestartStepKind =
  | 'stopDemi'
  | 'saveClouds'
  | 'keepDatabases'
  | 'switch'
  | 'startManager'
  | 'startDemi'
  | 'updateControl'
  | 'updateConversations'

export interface RestartStep {
  kind: RestartStepKind
  state: StepState
}

/** How far an update is, as `GET /api/update` says. */
export interface UpdateProgress {
  /** The version the update moves to. */
  to: string
  /**
   * The version it left, which it returns to when `to` does not start; none
   * for a start that only updates its data.
   */
  from?: string | null
  /** `to` did not start, and the server returns to `from`. */
  returning: boolean
  steps: readonly RestartStep[]
}

/** How long each step usually takes, which its share of the bar follows. */
const WEIGHTS: Record<RestartStepKind, number> = {
  stopDemi: 3,
  saveClouds: 25,
  keepDatabases: 10,
  switch: 1,
  startManager: 5,
  startDemi: 3,
  updateControl: 5,
  updateConversations: 48,
}

/**
 * The screen over the whole app from the moment Demi restarts for an update
 * until it serves again: every step of the rest of the update, each with what
 * it did or how far it is, and a bar of the whole. It shows counts and sizes,
 * never a name: it shows before anyone signs in. While Demi cannot be
 * reached it keeps the list it last read, so it never empties or changes
 * size; before Demi first says how far it is, the bar moves. Nothing on it
 * acts: the page loads itself once Demi serves again.
 */
const props = defineProps<{
  /** How far the update is; null until Demi says, which shows the bar moving. */
  progress: UpdateProgress | null
}>()

const now = ref(Date.now())
const ticking = setInterval(() => (now.value = Date.now()), 1000)
onUnmounted(() => clearInterval(ticking))

function progressDetail(progress: StepProgress): string {
  switch (progress.kind) {
    case 'bytes':
      return t('updating.bytes', { done: formatBytes(progress.done), all: formatBytes(progress.total) })
    case 'clouds':
    case 'devices':
      return t('updating.of', { done: progress.done, count: progress.total })
    case 'conversations':
      return `${t('updating.of', { done: progress.done, count: progress.total })} · ${t('updating.bytes', { done: formatBytes(progress.bytesDone), all: formatBytes(progress.bytes) })} · ${formatTimeLeft(progress.leftSeconds ?? null)}`
  }
}

function detail(state: StepState): string {
  switch (state.state) {
    case 'waiting':
      return ''
    case 'running':
      return state.progress ? progressDetail(state.progress) : formatSpan(now.value - Date.parse(state.started))
    case 'done':
      return state.summary.kind === 'bytes'
        ? `${formatBytes(state.summary.bytes)} · ${formatSpan(state.tookMs)}`
        : formatSpan(state.tookMs)
    case 'skipped':
      return t('updating.noChanges')
    case 'failed':
      return state.reason
  }
}

const lines = computed<StepLine[]>(() => (props.progress?.steps ?? []).map((step) => ({
  key: step.kind,
  title: t(`updating.steps.${step.kind}`),
  mark: step.state.state,
  detail: detail(step.state),
})))

/** A step failed after the stop, and nothing serves until an administrator finishes the update. */
const failed = computed(() => props.progress !== null && !props.progress.returning
  && props.progress.steps.some((step) => step.state.state === 'failed'))

const title = computed(() => {
  const progress = props.progress
  if (progress === null)
    return t('updating.title')
  if (progress.returning)
    return t('updating.returning', { version: progress.from ?? '' })
  return failed.value ? t('updating.failed', { version: progress.to }) : t('updating.to', { version: progress.to })
})

const overall = computed(() => props.progress ? overallDone(props.progress.steps, WEIGHTS) : 0)
</script>

<template>
  <div
    class="fixed inset-0 z-[100] flex flex-col items-center justify-center gap-5 bg-surface-base px-6"
    role="status"
    aria-live="polite"
  >
    <h1 class="text-center text-[15px] font-medium leading-5 text-fg-emphasis">{{ title }}</h1>
    <div class="w-full max-w-xl">
      <ProgressBar v-if="progress" :value="overall" :max="1" :label="title" />
      <div v-else class="relative h-1 w-full overflow-hidden rounded-full bg-overlay/8" role="progressbar" :aria-label="title">
        <div class="updating-moving absolute inset-y-0 w-1/3 rounded-full bg-accent-fill" />
      </div>
    </div>
    <!-- The list's room is every step's, before Demi first says how far it is too. -->
    <div class="h-56 w-full max-w-xl phone:h-96">
      <UpdateSteps :lines="lines" />
    </div>
    <!-- Room for the longer of the two sentences, so the screen keeps its place when a step fails. -->
    <p class="min-h-10 w-full max-w-xl text-center text-chrome text-fg-muted">{{ failed ? t('updating.failedBody') : t('updating.body') }}</p>
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
