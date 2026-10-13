<script setup lang="ts">
import { computed, watch, watchEffect } from 'vue'
import { useDocumentVisibility } from '@vueuse/core'
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import type { SettingsDevice } from '@demicodes/web-ui/settings/types'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { claimDevice, useInstallCommands } from '../devices/pairing'
import { useDeviceSettings } from './devices'
import { useSettingsAddress } from './address'
import { directStatus, measureDirect, tryDirect, watchDirect } from '../direct'
import { changeDeviceSchema } from '../api/generated/web-api'

const resources = useResources()
const product = useProduct()
const settings = useDeviceSettings()
const installCommands = useInstallCommands()
const address = useSettingsAddress()
const { cloud, resetError } = storeToRefs(settings)
const { revoke, rename, setRoute, resetCloud } = settings
/** The backend's limit on a device's name, as its schema states it. */
const nameMaxLength = changeDeviceSchema.shape.name.unwrap().unwrap().maxLength

/** Each paired device, with when it was paired and how this page reaches it. */
const devices = computed<SettingsDevice[]>(() =>
  resources.devices.map((device) => ({
    ...device,
    pairedAt: product.snapshot?.devices.find((candidate) => candidate.id === device.id)?.claimedAt ?? '',
    direct: directStatus(device.id),
  })),
)
// The list says how this page reaches each online device, so each is tried
// even before a conversation used it.
watchEffect(() => {
  for (const device of resources.devices) {
    if (device.state === 'online')
      watchDirect(device.id)
  }
})

/** The device whose page the address opens. */
const shown = computed(() => (address.section.value === 'devices' ? address.detail.value : null))
// A page of a device that went, revoked here or elsewhere, gives way to the list.
watch(
  [shown, () => product.snapshot],
  ([id, snapshot]) => {
    if (id && snapshot && !snapshot.devices.some((device) => device.id === id))
      void address.show('devices')
  },
)
const visibility = useDocumentVisibility()
// The shown device's paths are measured each time its page becomes visible
// with its runner connected: opened, or its browser tab brought to the front.
watch(
  () => {
    const id = shown.value
    const device = id ? resources.devices.find((candidate) => candidate.id === id) : undefined
    return visibility.value === 'visible' && device?.state === 'online' ? device.id : null
  },
  (id) => {
    if (id)
      measureDirect(id)
  },
  { immediate: true },
)
</script>

<template>
  <SettingsDevices
    :devices="devices"
    :shown="shown"
    :runner-release="product.snapshot?.runnerRelease ?? null"
    :projects="resources.projects"
    :load="product.load"
    :name-max-length="nameMaxLength"
    :cloud="cloud"
    :reset-error="resetError"
    :overlay-store="appOverlayStore"
    :install-commands="installCommands"
    :claim-device="claimDevice"
    @retry="product.reconnect"
    @reset-cloud="resetCloud"
    @show="address.show('devices', $event)"
    @set-route="setRoute"
    @try-now="tryDirect"
    @measure="measureDirect"
    @rename="rename"
    @revoke="revoke"
  />
</template>
