<script setup lang="ts">
import { computed, onBeforeUnmount } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsIntegrations from '@demicodes/web-ui/settings/SettingsIntegrations.vue'
import { useSettingsAddress } from './address'
import { useProduct } from '../state/product'
import { useIntegrationSettings } from './integrations'

const address = useSettingsAddress()
const product = useProduct()
const settings = useIntegrationSettings()

// A token is shown once: leaving the section forgets it.
onBeforeUnmount(() => settings.forgetTokens())
void settings.readServerSizes()

/** Add Integration: the new integration's page opens once it exists. */
async function add(): Promise<void> {
  const id = await settings.add()
  if (id)
    await address.show('integrations', id)
}

/** Delete Integration: the list shows again once it is gone. */
async function remove(id: string): Promise<void> {
  if (await settings.remove(id))
    await address.show('integrations', null)
}

/** The integration whose page the address opens. */
const shown = computed(() => (address.section.value === 'integrations' ? address.detail.value : null))
</script>

<template>
  <SettingsIntegrations
    :integrations="settings.integrations"
    :tokens="settings.tokens"
    :server-sizes="settings.serverSizes"
    :shown="shown"
    :load="product.listLoad('integrations')"
    :overlay-store="appOverlayStore"
    :on-add="add"
    :on-change="settings.change"
    :on-replace-token="settings.replaceToken"
    :on-delete="remove"
    @retry="product.retryList('integrations')"
    @show="address.show('integrations', $event)"
  />
</template>
