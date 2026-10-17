<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsAgent from '@demicodes/web-ui/settings/SettingsAgent.vue'
import { imageVendorRows } from '@demicodes/web-ui/settings/image-entries'
import { computed } from 'vue'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { useResources } from '../state/resources'

/**
 * Settings › Agent: the small model, the entry of each vendor's images and
 * the turn settings, each the user's preference, and what each Automatic
 * resolves to, which the catalog answers.
 */
const product = useProduct()
const preferences = usePreferences()
const resources = useResources()
const imageVendors = computed(() => imageVendorRows(resources.providers, product.automaticImageEntries))
</script>

<template>
  <SettingsAgent
    :overlay-store="appOverlayStore"
    :providers="resources.providerInfos"
    :models="resources.models"
    :catalog-models="resources.catalogModels"
    :automatic-model="product.automaticSmallModel?.model.name ?? null"
    :small-model="preferences.smallModel"
    @update:small-model="preferences.update({ smallModel: $event })"
    :same-provider="preferences.smallModelSameProvider"
    @update:same-provider="preferences.update({ smallModelSameProvider: $event })"
    :send-while-running="preferences.sendWhileRunning"
    @update:send-while-running="preferences.update({ sendWhileRunning: $event })"
    :resume-after-restart="preferences.resumeAfterRestart"
    @update:resume-after-restart="preferences.update({ resumeAfterRestart: $event })"
    :resume-at-retry="preferences.resumeAtRetry"
    @update:resume-at-retry="preferences.update({ resumeAtRetry: $event })"
    :image-vendors="imageVendors"
    :image-entries="preferences.imageEntries"
    @update:image-entries="preferences.update({ imageEntries: $event })"
  />
</template>
