<script setup lang="ts">
import { computed, ref, watch, watchEffect } from 'vue'
import { storeToRefs } from 'pinia'
import { useIntervalFn } from '@vueuse/core'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import type { SettingsDevice } from '@demicodes/web-ui/settings/types'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { claimDevice, useDeviceInstallation } from '../devices/pairing'
import { useDeviceSettings } from './devices'
import { useSettingsAddress } from './address'
import { directRoundTrip, directStatus, tryDirect, watchDirect } from '../direct'
import { changeDeviceSchema } from '../api/generated/web-api'

const resources = useResources()
const product = useProduct()
const settings = useDeviceSettings()
const installation = useDeviceInstallation()
const address = useSettingsAddress()
const { cloud, reset, revoking, renaming, changing } = storeToRefs(settings)
const { revoke, rename, setDirect, resetCloud } = settings
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

/** The shown device's direct round trip, measured while its page shows and its channel stands. */
const roundTripMs = ref<number | null>(null)
async function measure(): Promise<void> {
  const id = shown.value
  roundTripMs.value = id ? await directRoundTrip(id) : null
}
useIntervalFn(() => void measure(), 2000)
watch(
  () => [shown.value, shown.value ? directStatus(shown.value).connected : false],
  () => void measure(),
  { immediate: true },
)
</script>

<template>
  <SettingsDevices
    :devices="devices"
    :shown="shown"
    :runner-release="product.snapshot?.runnerRelease ?? null"
    :round-trip-ms="roundTripMs"
    :projects="resources.projects"
    :load="product.load"
    :changing-ids="changing"
    :renaming-ids="renaming"
    :revoking-ids="revoking"
    :name-max-length="nameMaxLength"
    :cloud="cloud"
    :reset-pending="reset.status === 'pending'"
    :reset-error="reset.status === 'failed' ? reset.message : null"
    :overlay-store="appOverlayStore"
    :installation="installation"
    :claim-device="claimDevice"
    @retry="product.reconnect"
    @reset-cloud="resetCloud"
    @show="address.show('devices', $event)"
    @set-direct="setDirect"
    @try-now="tryDirect"
    @rename="rename"
    @revoke="revoke"
  />
</template>
