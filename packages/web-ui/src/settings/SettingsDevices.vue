<script setup lang="ts">
import AsyncRegion from '../ui/AsyncRegion.vue'
import CloudSettings from '../cloud/CloudSettings.vue'
import type { CloudState } from '../cloud/types'
import { Monitor } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsDevice } from './types'
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
}>()
const emit = defineEmits<{
  retry: []
  revoke: [id: string]
  resetCloud: [operationId: string]
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
        <SettingsRow
          v-for="device in devices"
          :key="device.id"
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
              <span
                class="absolute -right-0.5 -top-0.5 size-1.5 rounded-full ring-2 ring-surface-float"
                :class="device.online ? 'bg-on-success' : 'bg-fg-ghost'"
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
