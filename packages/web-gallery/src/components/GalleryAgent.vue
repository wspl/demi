<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsAgent from '@demicodes/web-ui/settings/SettingsAgent.vue'
import { demoProviders } from '../fixtures/catalog'
import { galleryMessages } from '../fixtures/send-while-running'
import {
  galleryAutomaticModel,
  smallModelCatalog,
  type SmallModelState,
} from '../fixtures/small-model'

/**
 * Settings › Agent over a specimen's state: a choice shows at once, as the
 * product's preference does while its write is on its way, and Automatic
 * names the small model the backend would resolve. The Messages group
 * shares the gallery's turn settings with the composer.
 */
defineProps<{
  state: SmallModelState
}>()
</script>

<template>
  <SettingsAgent
    v-model:small-model="state.choice"
    v-model:same-provider="state.sameProvider"
    v-model:send-while-running="galleryMessages.sendWhileRunning"
    v-model:resume-after-restart="galleryMessages.resumeAfterRestart"
    v-model:resume-at-retry="galleryMessages.resumeAtRetry"
    :overlay-store="appOverlayStore"
    :providers="demoProviders"
    :models="smallModelCatalog"
    :catalog-models="smallModelCatalog"
    :automatic-model="galleryAutomaticModel(state)"
  />
</template>
