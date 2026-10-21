<script setup lang="ts">
import { t } from './locales/text'
import { watch } from 'vue'
import { storeToRefs } from 'pinia'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import ProviderLoginDialog from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import { useResources } from '../state/resources'
import { usePreferences } from '../state/preferences'
import { useProduct } from '../state/product'
import { useProviderSettings } from './providers'
import { useExposedPortSettings } from './exposed-ports'
import type { SettingsProviderEntry } from '@demicodes/web-ui/settings/types'

const resources = useResources()
const preferences = usePreferences()
const product = useProduct()
const settings = useProviderSettings()
const exposedPorts = useExposedPortSettings()
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

/** Expose Port on a failed test: the port is exposed, then the test runs again. */
async function exposeAndTest(provider: SettingsProviderEntry, deviceId: string, port: number): Promise<void> {
  if (await exposedPorts.expose(deviceId, port)) {
    test(provider)
  }
}

// The vendor catalog, which the page reads as it starts, fails in the Add
// Provider dialog's region, with its own Retry.
function loadVendors(): void {
  void product.loadVendors().catch(() => {})
}

// The model list's failure is its region's state: a load that fails again says so there.
function retryModels(): void {
  void product.loadModels(true).catch(() => {})
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
    :load="product.load"
    :vendor-load="product.vendorLoad"
    :model-load="product.catalogLoad"
    @retry="product.reconnect"
    @retry-vendors="loadVendors"
    @retry-models="retryModels"
    :vendors="resources.vendors"
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
    @expose-port="exposeAndTest"
    :is-exposed="exposedPorts.isExposed"
    :shared-device="exposedPorts.sharedDevice"
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
        ).catch((error: unknown) => report(t('providers.removeModelFailed'), error))
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
