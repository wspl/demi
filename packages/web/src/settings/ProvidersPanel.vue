<script setup lang="ts">
import { t } from './locales/text'
import { computed, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import type { SettingsCompatibleShapes } from '@demicodes/web-ui/settings/types'
import ProviderLoginDialog from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import { useResources } from '../state/resources'
import { usePreferences } from '../state/preferences'
import { useProduct } from '../state/product'
import { useProviderSettings } from './providers'
import { deviceMarkOf, hostKindOf, hostLabel } from '../state/hosts'
import type { HostMenuHost } from '@demicodes/web-ui/hosts/types'

/** The shapes before the catalog is read, while the page keeps their menus disabled: none besides None. */
const NO_SHAPES: SettingsCompatibleShapes = { image: [], video: [], audio: [] }

const resources = useResources()
const preferences = usePreferences()
const product = useProduct()
const settings = useProviderSettings()
const { providers, login } = storeToRefs(settings)
const {
  report,
  change,
  addProvider,
  addCompatible,
  removeProvider,
  beginLogin,
  closeLogin,
  test,
  refresh,
  refreshUsage,
  removeAccount,
  activateAccount,
  reorderAccounts,
  reorderProviders,
  saveModel,
  saveModels,
  searchModels,
  submitCode,
  loadCli,
  checkCli,
  installCli,
} = settings

/** The Hosts an entry may connect through, the Cloud first, as the Host menu lists them; none until the snapshot names them. */
const hosts = computed<HostMenuHost[] | undefined>(() => {
  const devices = product.snapshot?.devices
  return devices
    ?.map((device) => ({
      id: device.id,
      name: hostLabel(hostKindOf(device), device),
      kind: hostKindOf(device),
      state: device.state,
      mark: deviceMarkOf(device, devices),
    }))
    .sort((a, b) => Number(b.kind === 'cloud') - Number(a.kind === 'cloud'))
})

// The vendor catalog, which the page reads as it starts, fails in the Add
// Provider dialog's region, with its own Retry.
function loadVendors(): void {
  void product.loadVendors().catch(() => {})
}

function openUrl(url: string): void {
  window.open(url, '_blank', 'noopener,noreferrer')
}
// A process provider's newest CLI version is read when the page shows that
// provider set up; an install's progress and end arrive with the providers.
watch(
  () => {
    const shown = providers.value.find((provider) => provider.id === resources.selectedProviderId)
    return shown?.cli === undefined || shown.configured === false ? null : shown.id
  },
  () => {
    const shown = providers.value.find((provider) => provider.id === resources.selectedProviderId)
    if (shown) {
      loadCli(shown)
    }
  },
  { immediate: true },
)
</script>

<template>
  <SettingsProvidersPage
    v-model:model-editor="settings.modelEditor"
    v-model:selected-id="resources.selectedProviderId"
    v-model:detail-open="resources.providerDetailOpen"
    :providers="providers"
    :shared-api-keys="product.snapshot?.manages?.apiKeys === false"
    :load="product.load"
    :vendor-load="product.vendorLoad"
    :model-load="product.catalogLoad"
    @retry="product.reconnect"
    @retry-vendors="loadVendors"
    :vendors="resources.vendors"
    :compatible-shapes="product.vendors?.compatibleShapes ?? NO_SHAPES"
    :overlay-store="appOverlayStore"
    :refreshing-usage="settings.refreshingUsage"
    :usage-failures="settings.usageFailures"
    @change="change"
    @toggle-model="
      (provider, model, enabled) =>
        preferences.showModel(provider.id, model.id, enabled)
    "
    @add="addProvider"
    @add-compatible="addCompatible"
    @remove="removeProvider"
    @sign-in="beginLogin"
    @test="test"
    :hosts="hosts"
    @refresh="refresh"
    @refresh-usage="refreshUsage"
    @check-cli="checkCli"
    @install-cli="installCli"
    @activate-account="activateAccount"
    @reorder-accounts="reorderAccounts"
    @reorder-providers="reorderProviders"
    :multi-account-warned="preferences.multiAccountWarned"
    @multi-account-warned="preferences.update({ multiAccountWarned: true })"
    @remove-account="removeAccount"
    :save-model="saveModel"
    :search-models="searchModels"
    @remove-model="
      (provider, model) =>
        saveModels(
          provider,
          provider.models.filter((entry) => entry.id !== model.id),
        ).catch((error: unknown) => report('providers.removeModelFailed', t('providers.removeModelFailed'), error))
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
    @submit-code="submitCode"
  />
</template>
