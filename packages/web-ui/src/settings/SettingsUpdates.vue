<script setup lang="ts">
import { computed, onUnmounted, ref, watchEffect } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { formatBytes } from '../files/format'
import { t as shared } from '../locales/text'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import ExternalLink from '../ui/ExternalLink.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import ProgressBar from '../ui/ProgressBar.vue'
import UpdateSteps from '../ui/UpdateSteps.vue'
import { formatPast, formatSpan } from '../ui/dates'
import { formatTimeLeft } from '../ui/time-left'
import { currentStep, overallDone, type StepLine, type StepProgress, type StepState } from '../ui/update-steps'
import type { Action } from '../ui/operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import { t } from './locales/text'
import { updateOffered, type UpgradeChannel, type ServerUpgrade, type UpgradeStep, type UpgradeStepKind } from './server-upgrade'

/**
 * Settings › Updates, as macOS's: the release Demi runs on this
 * server, the channel it is offered releases from, and the newest one with
 * Update Now, which asks first. While an update runs, its row lists every
 * step it takes with what each did or how far it is, and a bar of the whole;
 * from its restart every page shows the update's screen. An update that
 * failed keeps its list, the failed step saying why, until the next one.
 *
 * Every state keeps the room of the others: the status row always has two
 * lines and one control at most, which fits beside them on a phone too, and
 * the list has every step from the start.
 */
const props = defineProps<{
  state: ServerUpgrade
  overlayStore: OverlayStore
  /** Changes the channel, which looks for the newest release of it. */
  onSetChannel?: Action<[UpgradeChannel]>
  /** Looks for the newest release now. */
  onCheck?: Action
  /** How many agents work on the server, every account's, which the question names. */
  onCountAgents?: () => Promise<number>
  /** Runs the update to the version offered. */
  onUpdate?: Action<[string]>
}>()

const CHANNELS: readonly UpgradeChannel[] = ['beta', 'canary']

/** How long each step usually takes, which its share of the bar follows. */
const WEIGHTS: Record<UpgradeStepKind, number> = {
  download: 25,
  unpack: 5,
  checkConfiguration: 1,
  downloadGvisor: 5,
  installImage: 10,
  checkDatabases: 2,
  restart: 7,
}

const running = computed(() => props.state.run.state === 'updating')

// A minute's tick says when the last check was; a running update counts
// its steps' seconds.
const now = ref(Date.now())
let ticking: ReturnType<typeof setInterval> | undefined
watchEffect(() => {
  clearInterval(ticking)
  ticking = setInterval(() => (now.value = Date.now()), running.value ? 1000 : 30_000)
})
onUnmounted(() => clearInterval(ticking))

const offered = computed(() => updateOffered(props.state))
/** The notes of the release offered. */
const notesUrl = computed(() => {
  const check = props.state.check
  return offered.value !== null && check.state === 'checked' ? (check.notesUrl ?? null) : null
})

const steps = computed<readonly UpgradeStep[]>(() => props.state.run.state === 'none' ? [] : props.state.run.steps)
const current = computed(() => currentStep(steps.value))

function stepTitle(kind: UpgradeStepKind, version: string): string {
  return t(`updates.steps.${kind}`, { version })
}

function progressDetail(progress: StepProgress): string {
  switch (progress.kind) {
    case 'bytes': {
      const size = t('updates.downloaded', { done: formatBytes(progress.done), all: formatBytes(progress.total) })
      if (progress.perSecond == null)
        return size
      if (progress.perSecond === 0)
        return `${size} · ${formatTimeLeft(null)}`
      const left = Math.max(0, progress.total - progress.done) / progress.perSecond
      return `${size} · ${t('updates.speed', { size: formatBytes(progress.perSecond) })} · ${formatTimeLeft(left)}`
    }
    case 'clouds':
    case 'conversations':
      return t('updates.detail.of', { done: progress.done, count: progress.total })
  }
}

function stateDetail(state: StepState): string {
  switch (state.state) {
    case 'waiting':
      return ''
    case 'running':
      return state.progress ? progressDetail(state.progress) : formatSpan(now.value - Date.parse(state.started))
    case 'done': {
      const took = formatSpan(state.tookMs)
      switch (state.summary.kind) {
        case 'none':
          return took
        case 'bytes':
          return `${formatBytes(state.summary.bytes)} · ${took}`
        case 'count':
          return `${t('updates.detail.toUpdate', { count: state.summary.count }, state.summary.count)} · ${took}`
      }
      return took
    }
    case 'skipped':
      return t(state.why === 'installed' ? 'updates.detail.installed' : 'updates.detail.noChanges')
    case 'failed':
      return state.reason
  }
}

const lines = computed<StepLine[]>(() => {
  const run = props.state.run
  const to = run.state === 'none' ? '' : run.to
  return steps.value.map((step) => ({
    key: step.kind,
    title: stepTitle(step.kind, to),
    mark: step.state.state,
    detail: stateDetail(step.state),
  }))
})

const overall = computed(() => overallDone(steps.value, WEIGHTS))

/**
 * The status row's words: what runs, what was found, or what failed. Every
 * state has both lines, so the row keeps its height as one replaces another.
 */
const status = computed<{ label: string, description: string }>(() => {
  const { run, check } = props.state
  const currentVersion = t('updates.current', { version: props.state.running })
  const youHave = t('updates.youHave', { version: props.state.running })
  if (run.state === 'updating') {
    const step = current.value
    if (step === null)
      return { label: t('updates.updatingTo', { version: run.to }), description: youHave }
    const description = t('updates.stepOf', { number: step.number, count: steps.value.length, step: stepTitle(step.step.kind, run.to) })
    return { label: t('updates.updatingTo', { version: run.to }), description }
  }
  if (run.state === 'failed') {
    const failed = current.value
    return {
      label: run.to
        ? t('updates.failed', { version: run.to })
        : t('updates.failedNoVersion', { running: props.state.running }),
      description: failed
        ? t('updates.failedAt', { number: failed.number, count: run.steps.length, step: stepTitle(failed.step.kind, run.to) })
        : run.reason,
    }
  }
  switch (check.state) {
    // The backend starts its first check as it starts, so a server that has
    // never checked is about to.
    case 'never':
    case 'checking':
      return { label: t('updates.checking'), description: currentVersion }
    case 'failed':
      return { label: t('updates.checkFailed'), description: check.reason }
    case 'checked':
      return check.newer
        ? { label: t('updates.available', { version: check.newer }), description: youHave }
        : { label: t('updates.upToDate'), description: currentVersion }
  }
  return { label: '', description: '' }
})

/** When the last check ended; a check that runs keeps the one before it. */
const lastCheckedAt = ref<string | null>(null)
watchEffect(() => {
  const check = props.state.check
  if (check.state === 'checked' || check.state === 'failed')
    lastCheckedAt.value = check.at
})
const lastChecked = computed(() => lastCheckedAt.value === null
  ? t('updates.neverChecked')
  : t('updates.lastChecked', { when: formatPast(lastCheckedAt.value, now.value) }))

const asking = ref(false)
/** Update Now counts the agents at work before it asks, so the question opens whole. */
const counting = ref(false)
/** The agents at work, which the question names. */
const agents = ref(0)
/** The version the question offers, kept while it shows. */
const asked = ref<string | null>(null)

async function ask(): Promise<void> {
  asked.value = offered.value
  counting.value = true
  try {
    agents.value = (await props.onCountAgents?.()) ?? 0
  } catch {
    // The question asks without the count.
    agents.value = 0
  } finally {
    counting.value = false
  }
  asking.value = true
}

function update(): unknown {
  const version = asked.value
  asking.value = false
  return version === null ? undefined : props.onUpdate?.(version)
}
</script>

<template>
  <SettingsPage :title="t('updates.title')" :description="t('updates.description')">
    <SettingsGroup>
      <SettingsRow :label="status.label" phone-stacked>
        <template #description>
          <span class="flex min-w-0 items-center gap-2 phone:min-h-hit-sm">
            <span class="min-w-0 truncate" :title="status.description">{{ status.description }}</span>
            <ExternalLink v-if="notesUrl" :href="notesUrl" class="shrink-0">{{ t('updates.releaseNotes') }}</ExternalLink>
          </span>
        </template>
        <!-- On a phone the control goes under the text, and its line stays when there is none. -->
        <div class="flex items-center justify-end phone:min-h-11 phone:w-full phone:justify-start">
          <div v-if="running" class="w-32 phone:w-full">
            <ProgressBar :value="overall" :max="1" :label="status.label" />
          </div>
          <Button v-else-if="offered" variant="primary" size="sm" :disabled="counting" @click="ask">
            {{ t('updates.updateNow') }}
          </Button>
        </div>
        <template v-if="lines.length" #below>
          <UpdateSteps :lines="lines" />
        </template>
      </SettingsRow>
      <SettingsRow :label="lastChecked">
        <Button
          size="sm"
          :disabled="running || state.check.state === 'checking'"
          :operation="{ action: 'check', target: 'updates' }"
          @click="onCheck?.()"
        >
          {{ t('updates.checkNow') }}
        </Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup>
      <SettingsRow :label="t('updates.channel')" :description="t('updates.channelDescription')">
        <Dropdown
          size="sm"
          :overlay-store="overlayStore"
          variant="default"
          :trigger-label="t('updates.channel')"
          :disabled="running"
        >
          <template #trigger>{{ t(`updates.${state.channel}`) }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="channel in CHANNELS"
                :key="channel"
                :label="t(`updates.${channel}`)"
                choice
                :is-selected="state.channel === channel"
                @select="close(); channel === state.channel || onSetChannel?.(channel)"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
    </SettingsGroup>
    <Dialog
      :is-open="asking"
      :overlay-store="overlayStore"
      :label="t('updates.confirmTitle', { version: asked ?? '' })"
      @close="asking = false"
    >
      <div class="flex flex-col gap-4 px-5 pt-5">
        <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">{{ t('updates.confirmTitle', { version: asked ?? '' }) }}</h3>
        <div class="flex flex-col gap-3 text-explanation leading-5 text-fg-muted">
          <p>{{ t('updates.confirmBody') }}</p>
          <p v-if="agents > 0">{{ t('updates.confirmAgents', { count: agents }, agents) }}</p>
        </div>
      </div>
      <template #footer>
        <Button @click="asking = false">{{ shared('common.cancel') }}</Button>
        <Button variant="primary" @click="update">{{ t('updates.update') }}</Button>
      </template>
    </Dialog>
  </SettingsPage>
</template>
