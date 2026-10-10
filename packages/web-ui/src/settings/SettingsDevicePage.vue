<script setup lang="ts">
import { computed, ref } from 'vue'
import { Cloud } from '@lucide/vue'
import { upperFirst } from '@demicodes/utils'
import Button from '../ui/Button.vue'
import DetailsSheet from '../ui/DetailsSheet.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import type { StatusDotTone } from '../ui/StatusDot.vue'
import { formatDay, formatMoment, useTimeUntil } from '../composables/useRelativeTime'
import DeviceRenameDialog from '../devices/DeviceRenameDialog.vue'
import DeviceRevokeDialog from '../devices/DeviceRevokeDialog.vue'
import DeviceStartHint from '../devices/DeviceStartHint.vue'
import { useAsked } from '../devices/useAsked'
import {
  DEVICE_ROUTE_DESCRIPTION,
  DEVICE_ROUTE_LABEL,
  DIRECT_STAGE_LABEL,
  addressWithPort,
  connectedVia,
  directReason,
  pathSentence,
  pathsLatency,
  reasonSentence,
  shownAddress,
  type DeviceRoute,
  type DirectAddresses,
  type DirectPermission,
} from '../devices/direct'
import { RUNNER_STATE_LABEL, runnerState, systemName, type DeviceReport } from '../devices/report'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE } from '../devices/state'
import { DEVICE_ICON } from '../hosts/icons'
import type { OverlayStore } from '../overlay/overlayStore'
import type { SentenceText } from '../ui/ui-text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsDevice } from './types'

/**
 * A device's own page (`direct-channel.md` § What the user sees), which its
 * row in Devices opens. Its header names the device and its state, and an
 * offline device's header gives the command that starts its runner.
 * Connection is one group of rows, as macOS's Wi-Fi settings lay it out:
 * the route; P2P, while the device is online and its route allows a peer,
 * saying in a sentence how the P2P connection fares, with Try Again and
 * Details… for the last attempt's diagnostics; and the two paths' latency
 * from this browser's last measurement, with Measure. Device holds the
 * facts, with Rename…; Revoke… ends the page. The Cloud's page has no Connection.
 */
const props = defineProps<{
  /** A paired device, or the Cloud with what its runner reported. */
  page: { kind: 'device'; device: SettingsDevice } | { kind: 'cloud'; report: DeviceReport }
  /** The runner release the server's devices follow; null on a server without them. */
  runnerRelease: string | null
  /** The names of the projects on the device, which go with it. */
  projects?: readonly string[]
  nameMaxLength: number | null
  revoking?: boolean
  overlayStore: OverlayStore
}>()
const emit = defineEmits<{
  back: []
  setRoute: [route: DeviceRoute]
  tryNow: []
  measure: []
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

/**
 * The header's dot and its words. The dot is the device's state, as the
 * devices list's dot is, so an online device is green whichever path this
 * page takes; the words say the path.
 */
const status = computed<{ tone: StatusDotTone; words: SentenceText }>(() => {
  const shown = device.value
  if (!shown) {
    return { tone: 'muted', words: 'Connected via relay' }
  }
  const tone = DEVICE_STATE_TONE[shown.state]
  if (shown.state !== 'online') {
    return { tone, words: DEVICE_STATE_LABEL[shown.state] }
  }
  return { tone, words: direct.value ? connectedVia(direct.value) : 'Connected via relay' }
})

/** How to start the runner again, which the header gives while the device is offline. */
const offlineStart = computed(() => (device.value?.state === 'offline' ? (device.value.start ?? null) : null))

/**
 * The Latency row's value while the device is online: the paths compared
 * from the last measurement, which stay shown while the next runs;
 * Measuring… until the first ends; and No answer after one the device
 * answered none of.
 */
const latency = computed<SentenceText | null>(() => {
  if (!direct.value || !online.value) {
    return null
  }
  return pathsLatency(direct.value) ?? (direct.value.measuring ? 'Measuring…' : 'No answer')
})

const nextIn = useTimeUntil(() => direct.value?.nextAt ?? new Date().toISOString())

/** The P2P row is there while the device is online and its route allows a peer. */
const peerAllowed = computed(() => online.value && !!direct.value && direct.value.route !== 'server')

/**
 * The P2P row's subtitle: the reason in the design table's words, or
 * the device's address in use while connected; none before an attempt ended.
 */
const directSentence = computed<SentenceText | null>(() => {
  if (reason.value) {
    return reasonSentence(reason.value)
  }
  const inUse = attempt.value?.inUse
  return inUse ? pathSentence(inUse.address) : null
})

/** Try Again is there only while the page has no connected peer; one that stands but is slower needs no new attempt. */
const canTryAgain = computed(() => !direct.value?.peer)
const tryAgain = useAsked(() => direct.value?.trying ?? false, () => emit('tryNow'))
/** Measure shows loading only for the measurement the user asked for, never one Demi runs by itself. */
const measure = useAsked(() => direct.value?.measuring ?? false, () => emit('measure'))

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

/**
 * The browser's local network permission as the attempt saw it; blocked, it
 * says where to allow it, since that is the reason's remedy.
 */
function permission(seen: DirectPermission | null): string | string[] {
  if (seen === null) {
    return 'Not asked by this browser'
  }
  return seen === 'denied' ? [PERMISSION_LABEL.denied, 'Allow it in this site’s settings.'] : PERMISSION_LABEL[seen]
}

/** The last attempt's diagnostics, as the Details sheet lists them, ending with when Demi tries next. */
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
    { label: 'Paths', value: `${seen.pairs.tried} tried, ${seen.pairs.answered} answered` },
    ...(seen.inUse ? [{ label: 'Path in use', value: addressWithPort(seen.inUse) }] : []),
    { label: 'Local network access', value: permission(seen.permission) },
    ...(direct.value?.nextAt ? [{ label: 'Next attempt', value: upperFirst(nextIn.value) }] : []),
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
    :status-tone="status.tone"
    @back="emit('back')"
  >
    <template #icon>
      <component :is="DEVICE_ICON" v-if="device" :size="24" aria-hidden="true" />
      <Cloud v-else :size="24" aria-hidden="true" />
    </template>
    <template #status>
      {{ status.words }}<span v-if="device?.state === 'offline' && device.seen" class="text-fg-subtle"> · Last seen <RelativeTime :timestamp="device.seen" /></span>
    </template>
    <template v-if="offlineStart" #description>
      <DeviceStartHint sentence="Start Demi on the device:" :start="offlineStart" />
    </template>

    <SettingsGroup v-if="device" title="Connection">
      <SettingsRow label="Route" :description="DEVICE_ROUTE_DESCRIPTION[device.direct.route]">
        <Dropdown
          size="sm"
          :overlay-store="overlayStore"
          variant="default"
          trigger-label="Route"
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
      <SettingsRow v-if="peerAllowed" label="P2P" :description="directSentence ?? undefined">
        <Button v-if="canTryAgain" size="sm" :loading="tryAgain.loading.value" @click="tryAgain.click">Try Again</Button>
        <Button v-if="attempt" size="sm" @click="detailsOpen = true">Details…</Button>
      </SettingsRow>
      <SettingsRow v-if="latency" label="Latency">
        <span class="text-chrome text-fg-muted">{{ latency }}</span>
        <Button size="sm" :loading="measure.loading.value" @click="measure.click">Measure</Button>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup title="Device">
      <SettingsRow v-if="device" label="Name">
        <span class="min-w-0 truncate text-chrome text-fg-muted">{{ device.name }}</span>
        <Button size="sm" @click="renameOpen = true">Rename…</Button>
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
      <SettingsRow label="Revoke this device">
        <Button size="sm" variant="danger" :loading="revoking" @click="revokeOpen = true">Revoke…</Button>
      </SettingsRow>
    </SettingsGroup>

    <DetailsSheet
      :is-open="detailsOpen"
      :overlay-store="overlayStore"
      title="Connection Details"
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
