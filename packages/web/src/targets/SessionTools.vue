<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import SessionToolsMenu from '@demicodes/web-ui/hosts/SessionToolsMenu.vue'
import type { ExposeMenuEntry } from '@demicodes/web-ui/hosts/types'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { useDeviceSettings } from '../settings/devices'
import { pageTabKind } from '@demicodes/web-ui/agent/panel-kinds/page'
import { exposePageTab } from '@demicodes/web-ui/agent/panel-kinds/page-data'
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

/** An expose opens in a new `page` tab of this conversation's work panel. */
function openExpose(expose: ExposeMenuEntry): void {
  work.add(props.conversationId, pageTabKind.kind, exposePageTab(expose))
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
