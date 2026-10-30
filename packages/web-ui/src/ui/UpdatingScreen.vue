<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { formatBytes } from '../files/format'
import Button from './Button.vue'
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
  /**
   * Set on the running Clouds' step of an update that keeps them running for
   * the new version rather than saving them, which its title then says.
   */
  kept?: boolean
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

/**
 * How the update ended, once it did: the new version serves, or the server
 * runs the version it left again.
 */
export type UpdateEnd = 'updated' | 'returned'

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
 * Kept Clouds take as long as stopping Demi: the manager only records the
 * Clouds it leaves running, while saving them stops each one.
 */
const KEPT_WEIGHT = WEIGHTS.stopDemi

/**
 * The screen over the whole app from the moment Demi restarts for an update
 * until the user opens Demi again: every step of the rest of the update, each
 * with what it did or how far it is, and a bar of the whole. It shows counts
 * and sizes, never a name: it shows before anyone signs in. While Demi cannot
 * be reached it keeps the list it last read, so it never empties or changes
 * size; before Demi first says how far it is, the bar moves. Once the update
 * ended, Open Demi loads the page; nothing loads it by itself, so the user
 * sees every step end. The line under the list and the button keep their
 * room in every state.
 */
const props = defineProps<{
  /** How far the update is; null until Demi says, which shows the bar moving. */
  progress: UpdateProgress | null
  /** How the update ended; none while it runs. */
  ended?: UpdateEnd | null
}>()

defineEmits<{ open: [] }>()

const now = ref(Date.now())
const ticking = setInterval(() => (now.value = Date.now()), 1000)
onUnmounted(() => clearInterval(ticking))

function progressDetail(progress: StepProgress): string {
  switch (progress.kind) {
    case 'bytes':
      return t('updating.bytes', { done: formatBytes(progress.done), all: formatBytes(progress.total) })
    case 'clouds':
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

/**
 * How a step shows once the update ended, from what the screen last read:
 * done every one once Demi is updated, as when the backend that serves
 * started again since and no longer says what each did; and the one still
 * running failed once the server returned, as when the screen did not hear
 * which step failed. Such a step says nothing more.
 */
function shownMark(state: StepState): StepLine['mark'] | null {
  if (props.ended === 'updated' && state.state !== 'done' && state.state !== 'skipped')
    return 'done'
  if (props.ended === 'returned' && state.state === 'running')
    return 'failed'
  return null
}

const lines = computed<StepLine[]>(() => (props.progress?.steps ?? []).map((step) => {
  const ended = shownMark(step.state)
  return {
    key: step.kind,
    title: step.kept ? t('updating.steps.keepClouds') : t(`updating.steps.${step.kind}`),
    mark: ended ?? step.state.state,
    detail: ended ? '' : detail(step.state),
  }
}))

/** A step failed after the stop, and nothing serves until an administrator finishes the update. */
const failed = computed(() => props.progress !== null && !props.progress.returning
  && props.progress.steps.some((step) => step.state.state === 'failed'))

const title = computed(() => {
  const progress = props.progress
  if (props.ended === 'updated')
    return progress ? t('updating.updated', { version: progress.to }) : t('updating.updatedDemi')
  if (progress === null)
    return t('updating.title')
  if (props.ended === 'returned')
    return t('updating.failed', { version: progress.to })
  if (progress.returning)
    return t('updating.returning', { version: progress.from ?? '' })
  return failed.value ? t('updating.failed', { version: progress.to }) : t('updating.to', { version: progress.to })
})

const overall = computed(() => {
  if (props.ended === 'updated')
    return 1
  return props.progress ? overallDone(props.progress.steps, (step) => step.kept ? KEPT_WEIGHT : WEIGHTS[step.kind]) : 0
})

const body = computed(() => {
  switch (props.ended) {
    case 'updated':
      return ''
    case 'returned':
      return t('updating.returned', { version: props.progress?.from ?? '' })
    default:
      return failed.value ? t('updating.failedBody') : t('updating.body')
  }
})
</script>

<template>
  <div
    class="fixed inset-0 z-[100] flex flex-col items-center justify-center gap-5 bg-surface-base px-6"
    role="status"
    aria-live="polite"
  >
    <h1 class="text-center text-[15px] font-medium leading-5 text-fg-emphasis">{{ title }}</h1>
    <div class="w-full max-w-xl">
      <ProgressBar v-if="progress || ended === 'updated'" :value="overall" :max="1" :label="title" />
      <div v-else class="relative h-1 w-full overflow-hidden rounded-full bg-overlay/8" role="progressbar" :aria-label="title">
        <div class="updating-moving absolute inset-y-0 w-1/3 rounded-full bg-accent-fill" />
      </div>
    </div>
    <!-- The list's room is every step's, before Demi first says how far it is too. -->
    <div class="h-56 w-full max-w-xl phone:h-96">
      <UpdateSteps :lines="lines" />
    </div>
    <!-- Room for the longest sentence and the button in every state, so nothing moves when a step fails or the update ends. -->
    <p class="min-h-10 w-full max-w-xl text-center text-chrome text-fg-muted">{{ body }}</p>
    <Button
      variant="primary"
      :class="ended ? '' : 'invisible'"
      :aria-hidden="ended ? undefined : 'true'"
      :tabindex="ended ? undefined : -1"
      @click="$emit('open')"
    >
      {{ t('updating.open') }}
    </Button>
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
