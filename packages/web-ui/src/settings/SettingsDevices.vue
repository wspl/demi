<script setup lang="ts">
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
import type { SettingsDevice, SettingsExpose } from './types'
import DeviceExposes from './DeviceExposes.vue'
import type { DeviceInstallation } from '../devices/installation'
import type { OverlayStore } from '../overlay/overlayStore'
import DevicePairingDialog from '../devices/DevicePairingDialog.vue'
import { useDevicePairing, type PairingResult } from '../devices/pairing'

const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  pendingIds?: string[]
  cloud: CloudState | null
  resetPending?: boolean
  resetError?: string | null
  devices: SettingsDevice[]
  overlayStore: OverlayStore
  installation: DeviceInstallation
  claimDevice: (code: string, signal?: AbortSignal) => Promise<PairingResult>
  /** The instance's expose domain; null means the feature is off and no expose controls show. */
  exposeDomain: string | null
  /** Every expose of the user, as the snapshot lists them, soonest expiry first. */
  exposes: SettingsExpose[]
  /** The Cloud device's exposes; the host knows its device id. */
  cloudExposes: SettingsExpose[]
  /** Expose ids with a renew or remove request in flight. */
  exposePendingIds?: string[]
}>()
const emit = defineEmits<{
  retry: []
  revoke: [id: string]
  resetCloud: [operationId: string]
  renewExpose: [id: string]
  removeExpose: [id: string]
}>()
const { isOpen, phase, open, close, submit } = useDevicePairing(
  (code, signal) => props.claimDevice(code, signal),
)
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
      :exposes="exposeDomain === null ? undefined : cloudExposes"
      :pending-ids="exposePendingIds"
      @renew="emit('renewExpose', $event)"
      @remove="emit('removeExpose', $event)"
      @reset="emit('resetCloud', $event)"
    />
    <SettingsGroup>
      <template #header>
        <header class="flex items-center justify-between gap-3">
          <h3 class="text-[15px] font-medium leading-5 text-fg-emphasis">
            Your devices
          </h3>
          <Button size="sm" @click="open">Add device</Button>
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
          >
            <template #leading>
              <span class="relative flex">
                <Monitor :size="ICON_PX.in28" />
                <CornerDot
                  :tone="device.online ? 'success' : 'muted'"
                  ring="float"
                  :label="device.online ? 'Online' : 'Offline'"
                />
              </span>
            </template>
            <Button
              size="sm"
              :loading="pendingIds?.includes(device.id)"
              @click="emit('revoke', device.id)"
              >Revoke</Button
            >
          </SettingsRow>
          <DeviceExposes
            v-if="exposeDomain !== null"
            :exposes="exposes.filter((expose) => expose.deviceId === device.id)"
            :pending-ids="exposePendingIds"
            @renew="emit('renewExpose', $event)"
            @remove="emit('removeExpose', $event)"
          />
        <div
          v-if="!devices.length"
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          No devices connected.
        </div>
      </AsyncRegion>
    </SettingsGroup>
    <DevicePairingDialog
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
