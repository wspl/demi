<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import ProviderLoginDialog from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { useProviderSettings } from './providers'

const resources = useResources()
const product = useProduct()
const settings = useProviderSettings()
const { providers, testing, refreshing, login } = storeToRefs(settings)
const {
  report,
  change,
  addProvider,
  addEndpoint,
  removeProvider,
  beginLogin,
  closeLogin,
  test,
  refresh,
  accountAction,
  saveModel,
  saveModels,
  submitToken,
} = settings

function retry(): void {
  void product.revalidate()
  void product.loadVendors().catch(report)
}

function openUrl(url: string): void {
  window.open(url, '_blank', 'noopener,noreferrer')
}
onMounted(() => void product.loadVendors().catch(report))
onUnmounted(closeLogin)
</script>

<template>
  <SettingsProvidersPage
    v-model:model-editor="settings.modelEditor"
    :save-errors="settings.saveErrors"
    @retry-save="settings.retrySave"
    v-model:selected-id="resources.selectedProviderId"
    v-model:detail-open="resources.providerDetailOpen"
    :providers="providers"
    :load="product.load !== 'ready' ? product.load : product.vendorLoad"
    :vendor-load="product.vendorLoad"
    :model-load="product.catalogLoad"
    @retry="retry"
    @retry-vendors="product.loadVendors().catch(report)"
    :vendors="resources.vendors"
    :overlay-store="appOverlayStore"
    :operations="settings.operations"
    :testing="testing"
    :refreshing="refreshing"
    @change="change"
    @toggle-model="
      (provider, model, enabled) =>
        resources.hideModel(provider.id, model.id, enabled)
    "
    @add="addProvider"
    @add-endpoint="addEndpoint"
    @remove="removeProvider"
    @sign-in="beginLogin"
    @test="test"
    @refresh="refresh"
    @activate-account="
      (provider, id) => accountAction(provider, id, 'activate')
    "
    @remove-account="(provider, id) => accountAction(provider, id, 'remove')"
    :save-model="saveModel"
    @remove-model="
      (provider, model) =>
        saveModels(
          provider,
          provider.models.filter((entry) => entry.id !== model.id),
        ).catch(report)
    "
  />
  <ProviderLoginDialog
    v-if="login"
    :is-open="true"
    :overlay-store="appOverlayStore"
    :vendor-name="login.provider.name"
    :phase="login.phase"
    @close="closeLogin"
    @open="openUrl"
    @retry="beginLogin(login.provider)"
    @submit-token="submitToken"
  />
</template>
