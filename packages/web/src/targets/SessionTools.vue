<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import SessionToolsMenu from '@demicodes/web-ui/hosts/SessionToolsMenu.vue'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { useDeviceSettings } from '../settings/devices'
import { sessionToolsExposes } from './session-tools'

const product = useProduct()
const resources = useResources()
const settings = useDeviceSettings()
const { exposes, exposeDomain, exposePending } = storeToRefs(settings)
const entries = computed(() =>
  sessionToolsExposes(exposes.value, product.snapshot?.devices ?? []),
)
</script>

<template>
  <SessionToolsMenu
    :exposes="entries"
    :expose-domain="exposeDomain"
    :pending-ids="exposePending"
    @renew="settings.renewExpose"
    @remove="settings.removeExpose"
    @manage-devices="resources.openSettings('devices')"
  />
</template>
