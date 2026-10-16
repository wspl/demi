<script setup lang="ts">
import { t as settingsText } from './locales/text'
import { formatDecimal } from '../ui/numbers'
import { t as uiText } from '../locales/text'
import { t } from '../devices/locales/text'
import { computed, ref } from 'vue'
import { upperFirst } from '@demicodes/utils'
import { Cable, Server, WandSparkles } from '@lucide/vue'
import type { Component } from 'vue'
import Button from '../ui/Button.vue'
import DetailsSheet from '../ui/DetailsSheet.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import type { StatusDotTone } from '../ui/StatusDot.vue'
import { useTimeUntil } from '../composables/useRelativeTime'
import { formatDay, formatMoment } from '../ui/dates'
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
import { DEVICE_STATE_TONE, deviceStatusLabel } from '../devices/state'
import { hostIcon } from '../hosts/icons'
import DeviceIcon from '../devices/DeviceIcon.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { SentenceText } from '../ui/ui-text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { Action } from '../ui/operations'
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
  /** The directories of the projects on the device, which go with it. */
  projects?: readonly string[]
  nameMaxLength: number | null
  overlayStore: OverlayStore
  /** Revokes the device; the dialog's button works until it settles. */
  onRevoke?: Action
}>()
const emit = defineEmits<{
  back: []
  setRoute: [route: DeviceRoute]
  tryNow: []
  measure: []
  rename: [name: string]
}>()

const ROUTES: readonly DeviceRoute[] = ['automatic', 'direct', 'server']
const ROUTE_ICONS: Record<DeviceRoute, Component> = { automatic: WandSparkles, direct: Cable, server: Server }

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
    return { tone: 'muted', words: t('connected.relay') }
  }
  const tone = DEVICE_STATE_TONE[shown.state]
  if (shown.state !== 'online' || shown.downloadPercent != null) {
    return { tone, words: deviceStatusLabel(shown) }
  }
  return { tone, words: direct.value ? connectedVia(direct.value) : t('connected.relay') }
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
  return pathsLatency(direct.value) ?? (direct.value.measuring ? t('page.measuring') : t('page.noAnswer'))
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
  return ms < 1000 ? t('details.ms', { ms }) : t('details.s', { s: formatDecimal(ms / 1000, 1) })
}

/** One side's addresses, local and public, each as a person can use it. */
function addressLines(side: DirectAddresses): string[] {
  const list = (addresses: readonly string[]) => (addresses.length ? [...new Set(addresses.map(shownAddress))].join(', ') : t('details.none'))
  return [t('details.local', { list: list(side.local) }), t('details.public', { list: list(side.public) })]
}

/**
 * The browser's local network permission as the attempt saw it; blocked, it
 * says where to allow it, since that is the reason's remedy.
 */
function permission(seen: DirectPermission | null): string | string[] {
  if (seen === null) {
    return t('permission.notAsked')
  }
  return seen === 'denied' ? [t('permission.denied'), t('permission.allowInSite')] : t(`permission.${seen}`)
}

/** The last attempt's diagnostics, as the Details sheet lists them, ending with when Demi tries next. */
const details = computed(() => {
  const seen = attempt.value
  if (!seen) {
    return []
  }
  const ended =
    seen.outcome === 'busy'
      ? t('details.busy')
      : seen.outcome === 'dropped'
        ? t('details.dropped')
        : seen.stage
          ? DIRECT_STAGE_LABEL[seen.stage]
          : t('details.unknown')
  return [
    { label: t('details.ran'), value: formatMoment(seen.startedAt) },
    { label: t('details.took'), value: took(seen.durationMs) },
    { label: t('details.endedAt'), value: ended },
    { label: t('details.thisBrowser'), value: addressLines(seen.browser) },
    { label: t('details.theDevice'), value: addressLines(seen.device) },
    { label: t('details.paths'), value: t('details.answered', seen.pairs) },
    ...(seen.inUse ? [{ label: t('details.pathInUse'), value: addressWithPort(seen.inUse) }] : []),
    { label: t('details.localAccess'), value: permission(seen.permission) },
    ...(direct.value?.nextAt ? [{ label: t('details.nextAttempt'), value: upperFirst(nextIn.value) }] : []),
  ]
})

const detailsOpen = ref(false)
const renameOpen = ref(false)
const revokeOpen = ref(false)
/** Revoking the device, as an operation on it (`operations.ts`): Revoke… shows it while it runs, the page shown again included. */
const revoking = computed(() => ({ action: 'revoke', target: `device:${device.value?.id ?? ''}` }))
/** Revoke confirmed: once it is done, the question closes, if it still asks about this device. */
async function revoke(): Promise<void> {
  const id = device.value?.id
  try {
    await props.onRevoke?.()
  } finally {
    if (device.value?.id === id) {
      revokeOpen.value = false
    }
  }
}
</script>

<template>
  <SettingsPage
    :title="device?.name ?? t('page.cloud')"
    :back="settingsText('devices.title')"
    :status-tone="status.tone"
    @back="emit('back')"
  >
    <template #icon>
      <DeviceIcon v-if="device" :state="device.state" :mark="device.mark" :size="24" />
      <component :is="hostIcon({ kind: 'cloud' })" v-else :size="24" aria-hidden="true" />
    </template>
    <template #status>
      {{ status.words }}<span v-if="device?.state === 'offline' && device.seen" class="text-fg-subtle"> · {{ t('page.lastSeen') }} <RelativeTime :timestamp="device.seen" /></span>
    </template>
    <template v-if="offlineStart" #description>
      <DeviceStartHint :sentence="t('page.startSentence')" :start="offlineStart" />
    </template>

    <SettingsGroup v-if="device" :title="t('page.connection')">
      <SettingsRow :label="t('page.route')" :description="DEVICE_ROUTE_DESCRIPTION[device.direct.route]">
        <Dropdown
          size="sm"
          :overlay-store="overlayStore"
          variant="default"
          :trigger-label="t('page.route')"
        >
          <template #trigger>{{ DEVICE_ROUTE_LABEL[device.direct.route] }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                v-for="route in ROUTES"
                :key="route"
                :icon="ROUTE_ICONS[route]"
                :label="DEVICE_ROUTE_LABEL[route]"
                choice
                :is-selected="device.direct.route === route"
                @select="close(); route !== device.direct.route && emit('setRoute', route)"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow v-if="peerAllowed" :label="t('page.p2p')" :description="directSentence ?? undefined">
        <Button v-if="canTryAgain" size="sm" :loading="tryAgain.loading.value" @click="tryAgain.click">{{ uiText('common.retry') }}</Button>
        <Button v-if="attempt" size="sm" @click="detailsOpen = true">{{ t('page.details') }}</Button>
      </SettingsRow>
      <SettingsRow v-if="latency" :label="t('page.latency')">
        <span class="text-chrome text-fg-muted">{{ latency }}</span>
        <Button size="sm" :loading="measure.loading.value" @click="measure.click">{{ t('page.measure') }}</Button>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup :title="t('page.device')">
      <SettingsRow v-if="device" :label="t('page.name')">
        <span class="min-w-0 truncate text-chrome text-fg-muted">{{ device.name }}</span>
        <Button size="sm" @click="renameOpen = true">{{ t('page.rename') }}</Button>
      </SettingsRow>
      <SettingsRow :label="t('page.system')">
        <span class="text-chrome text-fg-muted">{{ system ?? t('page.notReported') }}</span>
      </SettingsRow>
      <SettingsRow
        :label="t('page.runner')"
        :description="runner === 'outdated' ? t('page.runnerUpdates') : undefined"
      >
        <span class="text-chrome text-fg-muted">{{ runner ? RUNNER_STATE_LABEL[runner] : t('page.notReported') }}</span>
      </SettingsRow>
      <SettingsRow v-if="device" :label="t('page.paired')">
        <span class="text-chrome text-fg-muted">{{ formatDay(device.pairedAt) }}</span>
      </SettingsRow>
    </SettingsGroup>

    <SettingsGroup v-if="device">
      <SettingsRow :label="t('page.revokeThis')">
        <Button size="sm" variant="danger" :operation="revoking" @click="revokeOpen = true">{{ t('page.revoke') }}</Button>
      </SettingsRow>
    </SettingsGroup>

    <DetailsSheet
      :is-open="detailsOpen"
      :overlay-store="overlayStore"
      :title="t('details.title')"
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
      :home="device?.home"
      :operation="revoking"
      @close="revokeOpen = false"
      @revoke="revoke"
    />
  </SettingsPage>
</template>
