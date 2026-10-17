<script setup lang="ts">
import { computed } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsAgent from '@demicodes/web-ui/settings/SettingsAgent.vue'
import { imageVendorRows } from '@demicodes/web-ui/settings/image-entries'
import type { SettingsProviderEntry } from '@demicodes/web-ui/settings/types'
import { demoProviders } from '../fixtures/catalog'
import { mockProviders } from '../fixtures/settings'
import { galleryMessages } from '../fixtures/send-while-running'
import {
  galleryAutomaticImageEntries,
  galleryAutomaticModel,
  smallModelCatalog,
  type SmallModelState,
} from '../fixtures/small-model'

/**
 * Settings › Agent over a specimen's state: a choice shows at once, as the
 * product's preference does while its write is on its way, and each
 * Automatic names what the backend would resolve for the gallery's
 * entries: the small model, and the entry of each vendor's images, from
 * the Settings › Providers entries given, the full Settings' own, or the
 * gallery's. The Messages group shares the gallery's turn settings with
 * the composer.
 */
const props = defineProps<{
  state: SmallModelState
  entries?: SettingsProviderEntry[]
}>()

const fixed = mockProviders()
const imageVendors = computed(() => {
  const entries = props.entries ?? fixed
  return imageVendorRows(entries, galleryAutomaticImageEntries(entries))
})
</script>

<template>
  <SettingsAgent
    v-model:small-model="state.choice"
    v-model:same-provider="state.sameProvider"
    v-model:send-while-running="galleryMessages.sendWhileRunning"
    v-model:resume-after-restart="galleryMessages.resumeAfterRestart"
    v-model:resume-at-retry="galleryMessages.resumeAtRetry"
    v-model:image-entries="state.imageEntries"
    :overlay-store="appOverlayStore"
    :providers="demoProviders"
    :models="smallModelCatalog"
    :catalog-models="smallModelCatalog"
    :automatic-model="galleryAutomaticModel(state)"
    :image-vendors="imageVendors"
  />
</template>
