<script setup lang="ts">
import { computed, ref } from 'vue'
import AsyncRegion from '../ui/AsyncRegion.vue'
import CloudSettings from '../cloud/CloudSettings.vue'
import type { CloudState } from '../cloud/types'
import { Monitor } from '@lucide/vue'
import CornerDot from '../ui/CornerDot.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsDevice, SettingsRowStatus } from './types'
import type { DeviceInstallation } from '../devices/installation'
import type { OverlayStore } from '../overlay/overlayStore'
import DevicePairingDialog from '../devices/DevicePairingDialog.vue'
import DeviceRevokeDialog from '../devices/DeviceRevokeDialog.vue'
import DeviceStartHint from '../devices/DeviceStartHint.vue'
import { useDevicePairing, type PairingResult } from '../devices/pairing'

const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  pendingIds?: string[]
  cloud: CloudState | null
  resetPending?: boolean
  resetError?: string | null
  devices: SettingsDevice[]
  /** The user's projects, each on a device; a revoked device's go with it. */
  projects?: readonly { deviceId: string; name: string }[]
  overlayStore: OverlayStore
  /** Null until the host knows where its backend serves the installers. */
  installation: DeviceInstallation | null
  claimDevice: (code: string, signal?: AbortSignal) => Promise<PairingResult>
}>()
const emit = defineEmits<{
  retry: []
  revoke: [id: string]
  resetCloud: [operationId: string]
}>()
/** The note beside a device this page reaches without the server in the middle. */
const connectedDirectly: SettingsRowStatus = {
  label: 'Connected directly',
  tone: 'success',
  detail: 'This page reaches the device without going through the server, so files and the browser view load faster.',
}
const { isOpen, phase, open, close, submit } = useDevicePairing(
  (code, signal) => props.claimDevice(code, signal),
)
/**
 * The device whose revocation the confirmation asks about; it stays while the
 * confirmation closes, so the closing dialog keeps its text.
 */
const confirming = ref<SettingsDevice | null>(null)
const confirmOpen = ref(false)
const confirmingProjects = computed(() =>
  (props.projects ?? [])
    .filter((project) => project.deviceId === confirming.value?.id)
    .map((project) => project.name),
)
function confirmRevoke(device: SettingsDevice) {
  confirming.value = device
  confirmOpen.value = true
}
function revoke() {
  confirmOpen.value = false
  if (confirming.value) {
    emit('revoke', confirming.value.id)
  }
}
</script>

<template>
  <SettingsPage
    title="Devices"
    description="Machines that can host a conversation's working directory."
  >
    <CloudSettings
      v-if="cloud"
      :reset-pending="resetPending"
      :reset-error="resetError"
      :cloud="cloud"
      :overlay-store="overlayStore"
      @reset="emit('resetCloud', $event)"
    />
    <SettingsGroup>
      <template #header>
        <header class="flex items-center justify-between gap-3">
          <h3 class="text-[15px] font-medium leading-5 text-fg-emphasis">
            Your Devices
          </h3>
          <Button size="sm" @click="open()">Add Device</Button>
        </header>
      </template>
      <AsyncRegion
        :state="load"
        label="Loading devices…"
        @retry="emit('retry')"
      >
        <template v-for="device in devices" :key="device.id">
          <SettingsRow
            :label="device.name"
            :description="
              device.online
                ? 'Online'
                : device.seen
                  ? `Last seen ${device.seen}`
                  : 'Offline'
            "
            :statuses="device.direct === 'connected' ? [connectedDirectly] : undefined"
          >
            <template #leading>
              <span class="relative flex">
                <Monitor :size="ICON_PX.in28" />
                <CornerDot
                  :tone="device.online ? 'success' : 'muted'"
                  :label="device.online ? 'Online' : 'Offline'"
                />
              </span>
            </template>
            <template
              v-if="device.direct === 'blocked' || (!device.online && device.start)"
              #detail
            >
              <div class="flex min-w-0 flex-col gap-2">
                <DeviceStartHint
                  v-if="!device.online && device.start"
                  :start="device.start"
                />
                <p v-if="device.direct === 'blocked'" class="text-[12px] leading-4 text-fg-subtle">
                  This browser blocks direct connections to devices on this computer and network, so
                  this device is reached through the server. To allow them, open this site’s settings
                  in the browser and allow Local network access.
                </p>
              </div>
            </template>
            <Button
              size="sm"
              :loading="pendingIds?.includes(device.id)"
              @click="confirmRevoke(device)"
              >Revoke</Button
            >
          </SettingsRow>
        </template>
        <div
          v-if="!devices.length"
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          No devices connected.
        </div>
      </AsyncRegion>
    </SettingsGroup>
    <DeviceRevokeDialog
      :is-open="confirmOpen"
      :overlay-store="overlayStore"
      :device="confirming?.name ?? ''"
      :projects="confirmingProjects"
      @close="confirmOpen = false"
      @revoke="revoke"
    />
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
