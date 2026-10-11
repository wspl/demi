<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsSubagents from '@demicodes/web-ui/settings/SettingsSubagents.vue'
import { modelInfo } from '../state/catalog'
import { usePreferences } from '../state/preferences'
import { useProduct } from '../state/product'
import { useResources } from '../state/resources'
import { useSubagentSettings } from './subagents'

const product = useProduct()
const resources = useResources()
const preferences = usePreferences()
const subagents = useSubagentSettings()
const { settings } = storeToRefs(subagents)
const { switchSubagents, setLimit, switchProfile, saveProfile, deleteProfile } = subagents

/**
 * Every model each entry's catalog lists, the ones this browser hides
 * included: a profile's model is the backend's to check, not this page's
 * menu's, which offers what the composer's does.
 */
const catalogModels = computed(() =>
  Object.fromEntries(
    product.catalog.map((provider) => [
      provider.providerId,
      provider.models.map((model) => modelInfo(model, preferences.contextLimit(provider.providerId, model.id))),
    ]),
  ),
)
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
    :catalog-models="catalogModels"
    :catalog-ready="product.catalogLoad === 'ready'"
    :overlay-store="appOverlayStore"
    :save-profile="saveProfile"
    :delete-profile="deleteProfile"
    @switch="switchSubagents"
    @set-limit="setLimit"
    @switch-profile="switchProfile"
    @context-limit="preferences.setContextLimit"
  />
</template>
