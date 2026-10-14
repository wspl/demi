<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsSubagents from '@demicodes/web-ui/settings/SettingsSubagents.vue'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { useSubagentSettings } from './subagents'

const product = useProduct()
const resources = useResources()
const subagents = useSubagentSettings()
const { settings } = storeToRefs(subagents)
const { switchSubagents, setLimit, switchProfile, saveProfile, deleteProfile } = subagents
</script>

<template>
  <SettingsSubagents
    v-if="settings"
    :enabled="settings.enabled"
    :max-live="settings.maxLive"
    :max-depth="settings.maxDepth"
    :profiles="settings.profiles"
    :providers="resources.providerInfos"
    :models="resources.models"
    :catalog-models="resources.catalogModels"
    :catalog-ready="product.catalogLoad === 'ready'"
    :overlay-store="appOverlayStore"
    :save-profile="saveProfile"
    :delete-profile="deleteProfile"
    @switch="switchSubagents"
    @set-limit="setLimit"
    @switch-profile="switchProfile"
  />
</template>
