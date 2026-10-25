<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
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
import { formatPast } from '../ui/dates'
import { formatTimeLeft } from '../ui/time-left'
import type { Action } from '../ui/operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import { t } from './locales/text'
import { updateOffered, type UpgradeChannel, type ServerUpgrade } from './server-upgrade'

/**
 * Settings › Updates, as macOS's: the release Demi runs on this
 * server, the channel it is offered releases from, and the newest one with
 * Update Now, which asks first. The page shows the update while it
 * downloads and prepares; from its restart every page shows the update's
 * screen. An update that failed says so, and why, until the next one.
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

const now = ref(Date.now())
const ticking = setInterval(() => (now.value = Date.now()), 30_000)
onUnmounted(() => clearInterval(ticking))

const offered = computed(() => updateOffered(props.state))
/** The notes of the release offered. */
const notesUrl = computed(() => {
  const check = props.state.check
  return offered.value !== null && check.state === 'checked' ? (check.notesUrl ?? null) : null
})
const running = computed(() => {
  const run = props.state.run
  return run.state === 'downloading' || run.state === 'preparing' || run.state === 'restarting'
})

/** The status row's words: what runs, what was found, or what failed. */
const status = computed<{ label: string, description?: string }>(() => {
  const { run, check } = props.state
  switch (run.state) {
    case 'downloading':
      return { label: t('updates.downloading', { version: run.to }) }
    case 'preparing':
      return { label: t('updates.preparing', { version: run.to }) }
    case 'restarting':
      return { label: t('updates.restarting') }
    case 'failed':
      return {
        label: run.to
          ? t('updates.failed', { version: run.to, running: props.state.running })
          : t('updates.failedNoVersion', { running: props.state.running }),
        description: run.reason,
      }
  }
  switch (check.state) {
    case 'checking':
      return { label: t('updates.checking') }
    case 'failed':
      return { label: t('updates.checkFailed'), description: check.reason }
    case 'checked':
      return check.newer
        ? { label: t('updates.available', { version: check.newer }), description: t('updates.youHave', { version: props.state.running }) }
        : { label: t('updates.upToDate'), description: t('updates.current', { version: props.state.running }) }
    case 'never':
      return { label: t('updates.current', { version: props.state.running }) }
  }
  return { label: '' }
})

const lastChecked = computed(() => {
  const check = props.state.check
  return check.state === 'checked' || check.state === 'failed'
    ? t('updates.lastChecked', { when: formatPast(check.at, now.value) })
    : t('updates.neverChecked')
})

const asking = ref(false)
/** The agents at work, which the question names once the host counted them. */
const agents = ref<number | null>(null)
/** The version the question offers, kept while it shows. */
const asked = ref<string | null>(null)

async function ask(): Promise<void> {
  asked.value = offered.value
  agents.value = null
  asking.value = true
  try {
    agents.value = (await props.onCountAgents?.()) ?? 0
  } catch {
    // The question asks without the count.
  }
}

function update(): unknown {
  const version = asked.value
  asking.value = false
  return version === null ? undefined : props.onUpdate?.(version)
}

/**
 * The bytes of a download, its rate and the time it has left, as Chrome's
 * downloads list them; the rate is unknown for its first second.
 */
function downloadProgress(run: Extract<ServerUpgrade['run'], { state: 'downloading' }>): string {
  const size = t('updates.downloaded', { done: formatBytes(run.bytesDone), all: formatBytes(run.bytes) })
  if (run.bytesPerSecond === 0)
    return `${size} · ${formatTimeLeft(null)}`
  const left = Math.max(0, run.bytes - run.bytesDone) / run.bytesPerSecond
  return `${size} · ${t('updates.speed', { size: formatBytes(run.bytesPerSecond) })} · ${formatTimeLeft(left)}`
}
</script>

<template>
  <SettingsPage :title="t('updates.title')" :description="t('updates.description')">
    <SettingsGroup>
      <SettingsRow :label="status.label" :description="status.description">
        <Button
          v-if="offered"
          variant="primary"
          size="sm"
          :disabled="running"
          @click="ask"
        >
          {{ t('updates.updateNow') }}
        </Button>
        <template v-if="state.run.state === 'downloading'" #below>
          <div class="flex flex-col gap-1.5">
            <ProgressBar :value="state.run.bytesDone" :max="state.run.bytes" :label="status.label" />
            <span class="text-chrome tabular-nums text-fg-muted">{{ downloadProgress(state.run) }}</span>
          </div>
        </template>
        <template v-else-if="notesUrl" #below>
          <ExternalLink :href="notesUrl">{{ t('updates.releaseNotes') }}</ExternalLink>
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
          <p v-if="agents">{{ t('updates.confirmAgents', { count: agents }, agents) }}</p>
        </div>
      </div>
      <template #footer>
        <Button @click="asking = false">{{ shared('common.cancel') }}</Button>
        <Button variant="primary" @click="update">{{ t('updates.update') }}</Button>
      </template>
    </Dialog>
  </SettingsPage>
</template>
