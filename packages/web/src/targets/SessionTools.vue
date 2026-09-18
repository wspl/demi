<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import SessionToolsMenu from '@demicodes/web-ui/hosts/SessionToolsMenu.vue'
import type { ExposeMenuEntry } from '@demicodes/web-ui/hosts/types'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { useDeviceSettings } from '../settings/devices'
import { useWorkPanel } from '../conversation/work'
import { sessionToolsExposes } from './session-tools'

const props = defineProps<{ conversationId: string }>()
const product = useProduct()
const resources = useResources()
const settings = useDeviceSettings()
const work = useWorkPanel()
const { exposes, exposePending } = storeToRefs(settings)
const entries = computed(() =>
  sessionToolsExposes(exposes.value, product.snapshot?.devices ?? []),
)

/** An expose opens in a new browser tab of this conversation's work panel. */
function openExpose(expose: ExposeMenuEntry): void {
  work.addBrowser(work.stateFor(props.conversationId), {
    url: expose.url,
    title: expose.address,
  })
}
</script>

<template>
  <SessionToolsMenu
    :exposes="entries"
    :pending-ids="exposePending"
    @open="openExpose"
    @renew="settings.renewExpose"
    @remove="settings.removeExpose"
    @manage-devices="resources.openSettings('devices')"
  />
</template>
