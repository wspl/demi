<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud, Monitor } from '@lucide/vue'
import Button from '../ui/Button.vue'
import DetailsSheet from '../ui/DetailsSheet.vue'
import Dropdown from '../ui/Dropdown.vue'
import FiguresTable from '../ui/FiguresTable.vue'
import HelpPopover from '../ui/HelpPopover.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import StatusDot from '../ui/StatusDot.vue'
import { formatDay, formatMoment, formatWhen, useTimeUntil } from '../composables/useRelativeTime'
import DeviceRenameDialog from '../devices/DeviceRenameDialog.vue'
import DeviceRevokeDialog from '../devices/DeviceRevokeDialog.vue'
import DeviceStartHint from '../devices/DeviceStartHint.vue'
import {
  DEVICE_ROUTE_DESCRIPTION,
  DEVICE_ROUTE_LABEL,
  DIRECT_STAGE_LABEL,
  directReason,
  formatLatency,
  formatLoss,
  reasonSentence,
  shownAddress,
  type DeviceRoute,
  type DirectAddresses,
  type PathFigures,
} from '../devices/direct'
import { RUNNER_STATE_LABEL, runnerState, systemName, type DeviceReport } from '../devices/report'
import { DEVICE_STATE_LABEL } from '../devices/state'
import type { OverlayStore } from '../overlay/overlayStore'
import type { SentenceText } from '../ui/ui-text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsDevice } from './types'

/**
 * A device's own page (`direct-channel.md` § What the user sees), which its
 * row in Devices opens. Its header says how this page reaches it now, why
 * not directly and when Demi tries again, with Details… for the last
 * attempt's diagnostics and Try Again; Connection holds the route, with the
 * two paths' measured figures under it and Test Speed; Device holds the
 * facts, with Rename…; Revoke… ends the page. The Cloud's page says it is
 * always reached through the server and has no Connection.
 */
const props = defineProps<{
  /** A paired device, or the Cloud with what its runner reported. */
  page: { kind: 'device'; device: SettingsDevice } | { kind: 'cloud'; report: DeviceReport }
  /** The runner release the server's devices follow; null on a server without them. */
  runnerRelease: string | null
  /** The names of the projects on the device, which go with it. */
  projects?: readonly string[]
  nameMaxLength: number | null
  /** A change of the device's route is under way. */
  changing?: boolean
  renaming?: boolean
  revoking?: boolean
  overlayStore: OverlayStore
}>()
const emit = defineEmits<{
  back: []
  setRoute: [route: DeviceRoute]
  tryNow: []
  testSpeed: []
  rename: [name: string]
  revoke: []
}>()

const ROUTES: readonly DeviceRoute[] = ['automatic', 'direct', 'server']

const device = computed(() => (props.page.kind === 'device' ? props.page.device : null))
const report = computed<DeviceReport>(() => (props.page.kind === 'device' ? props.page.device : props.page.report))
const system = computed(() => systemName(report.value))
const runner = computed(() => runnerState(report.value.runnerVersion, props.runnerRelease))

const direct = computed(() => device.value?.direct ?? null)
const online = computed(() => device.value?.state === 'online')
const reason = computed(() => (direct.value && online.value ? directReason(direct.value) : null))
const attempt = computed(() => direct.value?.attempt ?? null)

/** The figures of the path in use. */
const inUse = computed<PathFigures | null>(() => {
  const status = direct.value
  if (!status) {
    return null
  }
  return status.chosen ? status.figures.direct : status.figures.relay
})

/** The header's dot and its words: how this page reaches the device now. */
const status = computed<{ tone: 'success' | 'warning' | 'muted'; words: SentenceText }>(() => {
  const shown = device.value
  if (!shown) {
    return { tone: 'muted', words: 'Through the server' }
  }
  if (shown.state === 'updating') {
    return { tone: 'warning', words: DEVICE_STATE_LABEL.updating }
  }
  if (shown.state === 'offline') {
    return { tone: 'muted', words: DEVICE_STATE_LABEL.offline }
  }
  return reason.value === null
    ? { tone: 'success', words: 'Connected directly' }
    : { tone: 'muted', words: 'Through the server' }
})

const nextIn = useTimeUntil(() => direct.value?.nextAt ?? new Date().toISOString())

/** The sentence under the status: why the server's path is used, then when Demi tries again. */
const explanation = computed<SentenceText | null>(() => {
  const status = direct.value
  if (!status || !online.value) {
    return null
  }
  if (status.trying && reason.value?.kind === 'notYet') {
    return 'Demi is trying a direct connection.'
  }
  const sentence = reason.value ? reasonSentence(reason.value, formatWhen) : null
  if (!sentence) {
    return null
  }
  const retries = reason.value?.kind !== 'serverOnly' && reason.value?.kind !== 'slower' && status.nextAt !== null
  return retries ? `${sentence} Demi tries again ${nextIn.value}.` : sentence
})

/** Try Again is there only while the route allows a peer and the page is not connected directly. */
const canTryAgain = computed(() => {
  const status = direct.value
  return !!status && online.value && status.route !== 'server' && !status.chosen
})

const columns = [
  { key: 'latency', label: 'Latency' },
  { key: 'jitter', label: 'Jitter' },
  { key: 'loss', label: 'Loss' },
  { key: 'speed', label: 'Speed' },
] as const

/** A path's figures as the table shows them. */
function figureValues(figures: PathFigures | null, speed: number | null): Record<string, string | null> {
  return {
    latency: figures ? formatLatency(figures.latencyMs) : null,
    jitter: figures ? formatLatency(figures.jitterMs) : null,
    loss: figures?.loss === null || figures?.loss === undefined ? null : formatLoss(figures.loss),
    speed: speed === null ? null : `${speed < 10 ? speed.toFixed(1) : Math.round(speed)} MiB/s`,
  }
}

const rows = computed(() => {
  const status = direct.value
  if (!status) {
    return []
  }
  return [
    { key: 'direct', label: 'Direct', values: figureValues(status.figures.direct, status.speed.direct), current: status.chosen },
    { key: 'server', label: 'Server', values: figureValues(status.figures.relay, status.speed.relay), current: !status.chosen },
  ]
})

/** How long an attempt took, as a person reads it. */
function took(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`
}

/** One side's addresses, local and public, each as a person can use it. */
function addressLines(side: DirectAddresses): string[] {
  const list = (addresses: readonly string[]) => (addresses.length ? [...new Set(addresses.map(shownAddress))].join(', ') : 'None')
  return [`Local: ${list(side.local)}`, `Public: ${list(side.public)}`]
}

const PERMISSION_LABEL = {
  granted: 'Allowed',
  prompt: 'Not answered yet',
  denied: 'Blocked',
} as const

/** The last attempt's diagnostics, as the Details sheet lists them. */
const details = computed(() => {
  const seen = attempt.value
  if (!seen) {
    return []
  }
  const ended =
    seen.outcome === 'busy'
      ? 'The device was busy'
      : seen.outcome === 'dropped'
        ? 'Connected, then dropped'
        : seen.stage
          ? DIRECT_STAGE_LABEL[seen.stage]
          : 'Not known'
  return [
    { label: 'Ran', value: formatMoment(seen.startedAt) },
    { label: 'Took', value: took(seen.durationMs) },
    { label: 'Ended at', value: ended },
    { label: 'This browser', value: addressLines(seen.browser) },
    { label: 'The device', value: addressLines(seen.device) },
    { label: 'Paths tried', value: `${seen.pairs.tried}, of which ${seen.pairs.answered} answered` },
    ...(seen.pair
      ? [{ label: 'Path in use', value: `${seen.pair.browser === null ? 'Hidden by the browser' : shownAddress(seen.pair.browser)} to ${seen.pair.device}` }]
      : []),
    { label: 'Local network access', value: seen.permission ? PERMISSION_LABEL[seen.permission] : 'Not asked by this browser' },
  ]
})

const detailsOpen = ref(false)
const renameOpen = ref(false)
const revokeOpen = ref(false)
function revoke() {
  revokeOpen.value = false
  emit('revoke')
}
</script>

<template>
  <SettingsPage
    :title="device?.name ?? 'Cloud'"
    back="Devices"
    @back="emit('back')"
  >
    <template #icon>
      <Monitor v-if="device" :size="20" aria-hidden="true" />
      <Cloud v-else :size="20" aria-hidden="true" />
    </template>
    <template #status>
      <StatusDot :tone="status.tone" />
      <span>{{ status.words }}</span>
      <template v-if="device?.state === 'offline' && device.seen">
        <span class="text-fg-subtle">· Last seen <RelativeTime :timestamp="device.seen" /></span>
      </template>
      <span v-else-if="online && inUse" class="text-fg-subtle">· {{ formatLatency(inUse.latencyMs) }}</span>
    </template>
    <template v-if="!device || explanation" #description>
      {{ device ? explanation : 'The Cloud runs beside the server, so this browser always reaches it through the server.' }}
    </template>
    <template v-if="attempt || canTryAgain || (device?.state === 'offline' && device.start)" #actions>
      <HelpPopover
        v-if="device?.state === 'offline' && device.start"
        label="How to Start Its Runner"
        :overlay-store="overlayStore"
      >
        <DeviceStartHint :start="device.start" />
      </HelpPopover>
      <Button v-if="attempt" size="sm" @click="detailsOpen = true">Details…</Button>
      <Button v-if="canTryAgain" size="sm" :loading="direct?.trying" @click="emit('tryNow')">Try Again</Button>
    </template>

    <div v-if="device" class="flex flex-col gap-3">
      <SettingsGroup title="Connection">
        <SettingsRow label="Route" :description="DEVICE_ROUTE_DESCRIPTION[device.direct.route]">
          <Dropdown
            size="sm"
            :overlay-store="overlayStore"
            variant="default"
            trigger-label="Route"
            :disabled="changing"
          >
            <template #trigger>{{ DEVICE_ROUTE_LABEL[device.direct.route] }}</template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  v-for="route in ROUTES"
                  :key="route"
                  :label="DEVICE_ROUTE_LABEL[route]"
                  choice
                  :is-selected="device.direct.route === route"
                  @select="close(); route !== device.direct.route && emit('setRoute', route)"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
      </SettingsGroup>
      <FiguresTable
        v-if="online"
        caption="Measured from this browser"
        :columns="columns"
        :rows="rows"
        current-label="In use"
      >
        <template #action>
          <Button size="sm" :loading="device.direct.speed.testing" @click="emit('testSpeed')">Test Speed</Button>
        </template>
      </FiguresTable>
    </div>

    <SettingsGroup title="Device">
      <SettingsRow v-if="device" label="Name">
        <span class="min-w-0 truncate text-chrome text-fg-muted">{{ device.name }}</span>
        <Button size="sm" :loading="renaming" @click="renameOpen = true">Rename…</Button>
      </SettingsRow>
      <SettingsRow label="System">
        <span class="text-chrome text-fg-muted">{{ system ?? 'Not reported yet' }}</span>
      </SettingsRow>
      <SettingsRow
        label="Runner"
        :description="runner === 'outdated' ? 'It updates itself the next time it connects.' : undefined"
      >
        <span class="text-chrome text-fg-muted">{{ runner ? RUNNER_STATE_LABEL[runner] : 'Not reported yet' }}</span>
      </SettingsRow>
      <SettingsRow v-if="device" label="Paired">
        <span class="text-chrome text-fg-muted">{{ formatDay(device.pairedAt) }}</span>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup v-if="device">
      <SettingsRow
        label="Revoke this device"
        description="It leaves your devices, and its runner removes itself if it is connected."
      >
        <Button size="sm" variant="danger" :loading="revoking" @click="revokeOpen = true">Revoke…</Button>
      </SettingsRow>
    </SettingsGroup>

    <DetailsSheet
      :is-open="detailsOpen"
      :overlay-store="overlayStore"
      title="Connection Details"
      description="What the last attempt at a direct connection saw."
      :entries="details"
      @close="detailsOpen = false"
    />
    <DeviceRenameDialog
      :is-open="renameOpen"
      :overlay-store="overlayStore"
      :name="device?.name ?? ''"
      :max-length="nameMaxLength"
      @close="renameOpen = false"
      @rename="emit('rename', $event)"
    />
    <DeviceRevokeDialog
      :is-open="revokeOpen"
      :overlay-store="overlayStore"
      :device="device?.name ?? ''"
      :projects="projects ?? []"
      @close="revokeOpen = false"
      @revoke="revoke"
    />
  </SettingsPage>
</template>
