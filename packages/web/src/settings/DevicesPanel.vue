<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { claimDevice, deviceInstallation } from '../devices/pairing'
import { useDeviceSettings } from './devices'
const resources = useResources()
const product = useProduct()
const settings = useDeviceSettings()
const { cloud, reset, revoking } = storeToRefs(settings)
const { revoke, resetCloud } = settings
</script>

<template>
  <SettingsDevices
    :devices="resources.devices"
    :load="product.load"
    :pending-ids="revoking"
    @retry="product.revalidate"
    :cloud="cloud"
    :reset-pending="reset.status === 'pending'"
    :reset-error="reset.status === 'failed' ? reset.message : null"
    @reset-cloud="resetCloud"
    :overlay-store="appOverlayStore"
    :installation="deviceInstallation"
    :claim-device="claimDevice"
    @revoke="revoke"
  />
</template>
