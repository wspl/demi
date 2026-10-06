<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { claimDevice, useDeviceInstallation } from '../devices/pairing'
import { useDeviceSettings } from './devices'
import { directNote } from '../direct'
const resources = useResources()
const product = useProduct()
const settings = useDeviceSettings()
const installation = useDeviceInstallation()
const { cloud, reset, revoking } = storeToRefs(settings)
const { revoke, resetCloud } = settings
/** Each paired device, with what this page's path to it is. */
const devices = computed(() =>
  resources.devices.map((device) => ({ ...device, direct: directNote(device.id) })),
)
</script>

<template>
  <SettingsDevices
    :devices="devices"
    :projects="resources.projects"
    :load="product.load"
    :pending-ids="revoking"
    @retry="product.reconnect"
    :cloud="cloud"
    :reset-pending="reset.status === 'pending'"
    :reset-error="reset.status === 'failed' ? reset.message : null"
    @reset-cloud="resetCloud"
    :overlay-store="appOverlayStore"
    :installation="installation"
    :claim-device="claimDevice"
    @revoke="revoke"
  />
</template>
