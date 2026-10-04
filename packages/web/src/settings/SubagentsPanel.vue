<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsSubagents from '@demicodes/web-ui/settings/SettingsSubagents.vue'
import { modelInfo } from '../state/catalog'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { useSubagentSettings } from './subagents'

const product = useProduct()
const resources = useResources()
const subagents = useSubagentSettings()
const { settings, switching, pending } = storeToRefs(subagents)
const { switchSubagents, switchProfile, saveProfile, deleteProfile } = subagents

/**
 * Every model each entry's catalog lists, the ones this browser hides
 * included: a profile's model is the backend's to check, not this page's
 * menu's.
 */
const catalogModels = computed(() =>
  Object.fromEntries(product.catalog.map((provider) => [provider.providerId, provider.models.map(modelInfo)])),
)
</script>

<template>
  <SettingsSubagents
    v-if="settings"
    :enabled="settings.enabled"
    :profiles="settings.profiles"
    :providers="resources.providerInfos"
    :models="catalogModels"
    :catalog-ready="product.catalogLoad === 'ready'"
    :overlay-store="appOverlayStore"
    :switching="switching"
    :pending="pending"
    :save-profile="saveProfile"
    :delete-profile="deleteProfile"
    @switch="switchSubagents"
    @switch-profile="switchProfile"
  />
</template>
