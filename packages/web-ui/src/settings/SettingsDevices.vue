<script setup lang="ts">
import { computed } from 'vue'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import type { SentenceText } from '../ui/ui-text'
import CloudSettings from '../cloud/CloudSettings.vue'
import type { CloudState } from '../cloud/types'
import DevicePairingDialog from '../devices/DevicePairingDialog.vue'
import { directReason, type DeviceRoute } from '../devices/direct'
import type { DeviceInstallation } from '../devices/installation'
import { useDevicePairing, type PairingResult } from '../devices/pairing'
import DeviceIcon from '../devices/DeviceIcon.vue'
import { DEVICE_STATE_LABEL } from '../devices/state'
import type { OverlayStore } from '../overlay/overlayStore'
import SettingsDevicePage from './SettingsDevicePage.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsDevice } from './types'

/**
 * Settings → Devices (`direct-channel.md` § What the user sees): the Cloud,
 * then the paired devices, each a row that names it and says in one line
 * its system and its state, such as how this page reaches it or when an
 * offline one was last seen, and opens its own page, which `shown` names;
 * the page is shown in the list's place.
 */
const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  cloud: CloudState | null
  resetPending?: boolean
  resetError?: string | null
  devices: SettingsDevice[]
  /** The device whose page is shown, the Cloud's included; null shows the list. */
  shown?: string | null
  /** The runner release the server's devices follow; null on a server without them. */
  runnerRelease: string | null
  /** The devices whose change of their route, name or revocation is under way. */
  changingIds?: string[]
  renamingIds?: string[]
  revokingIds?: string[]
  /** The most characters a device's name has; null for no limit. */
  nameMaxLength: number | null
  /** The user's projects, each on a device; a revoked device's go with it. */
  projects?: readonly { deviceId: string; name: string }[]
  overlayStore: OverlayStore
  /** Null until the host knows where its backend serves the installers. */
  installation: DeviceInstallation | null
  claimDevice: (code: string, signal?: AbortSignal) => Promise<PairingResult>
}>()
const emit = defineEmits<{
  retry: []
  /** Shows a device's page, or the list again with null. */
  show: [id: string | null]
  setRoute: [id: string, route: DeviceRoute]
  tryNow: [id: string]
  rename: [id: string, name: string]
  revoke: [id: string]
  resetCloud: [operationId: string]
}>()

/** A device's state in its row: how this page reaches it while it is online; the reason waits on its page. */
function reachedAs(device: SettingsDevice): SentenceText {
  if (device.state !== 'online') {
    return DEVICE_STATE_LABEL[device.state]
  }
  return directReason(device.direct) === null ? 'Connected via P2P' : 'Connected via relay'
}

const { isOpen, phase, open, close, submit } = useDevicePairing(
  (code, signal) => props.claimDevice(code, signal),
)

/** The page shown in the list's place: a paired device's, the Cloud's, or none. */
const page = computed(() => {
  const shown = props.shown
  if (!shown) {
    return null
  }
  const device = props.devices.find((candidate) => candidate.id === shown)
  if (device) {
    return { kind: 'device' as const, device }
  }
  if (props.cloud && props.cloud.deviceId === shown) {
    return { kind: 'cloud' as const, report: props.cloud.report }
  }
  return null
})
const pageProjects = computed(() =>
  (props.projects ?? [])
    .filter((project) => project.deviceId === props.shown)
    .map((project) => project.name),
)
</script>

<template>
  <SettingsDevicePage
    v-if="page"
    :key="shown ?? ''"
    :page="page"
    :runner-release="runnerRelease"
    :projects="pageProjects"
    :name-max-length="nameMaxLength"
    :changing="!!shown && changingIds?.includes(shown)"
    :renaming="!!shown && renamingIds?.includes(shown)"
    :revoking="!!shown && revokingIds?.includes(shown)"
    :overlay-store="overlayStore"
    @back="emit('show', null)"
    @set-route="emit('setRoute', shown!, $event)"
    @try-now="emit('tryNow', shown!)"
    @rename="emit('rename', shown!, $event)"
    @revoke="emit('revoke', shown!)"
  />
  <SettingsPage
    v-else
    title="Devices"
    description="Machines that can host a conversation’s working directory."
  >
    <CloudSettings
      v-if="cloud"
      :reset-pending="resetPending"
      :reset-error="resetError"
      :cloud="cloud"
      :overlay-store="overlayStore"
      @reset="emit('resetCloud', $event)"
      @open="cloud.deviceId && emit('show', cloud.deviceId)"
    />
    <SettingsGroup title="Your Devices">
      <template #actions>
        <Button size="sm" @click="open()">Add Device…</Button>
      </template>
      <AsyncRegion
        :state="load"
        label="Loading devices…"
        @retry="emit('retry')"
      >
        <SettingsRow
          v-for="device in devices"
          :key="device.id"
          :label="device.name"
          navigable
          @click="emit('show', device.id)"
        >
          <template #description>
            <template v-if="device.os">{{ device.os.name }} · </template>{{ reachedAs(device) }}<template v-if="device.state === 'offline' && device.seen"> · Last seen <RelativeTime :timestamp="device.seen" /></template>
          </template>
          <template #leading>
            <DeviceIcon :state="device.state" />
          </template>
        </SettingsRow>
        <div
          v-if="!devices.length"
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          No devices connected.
        </div>
      </AsyncRegion>
    </SettingsGroup>
    <DevicePairingDialog
      v-if="installation"
      :is-open="isOpen"
      :overlay-store="overlayStore"
      :installation="installation"
      :phase="phase"
      @close="close"
      @next="phase = { kind: 'code' }"
      @back="phase = { kind: 'setup' }"
      @submit="submit"
    />
  </SettingsPage>
</template>
