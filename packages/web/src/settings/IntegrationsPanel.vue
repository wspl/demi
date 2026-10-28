<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsIntegrations from '@demicodes/web-ui/settings/SettingsIntegrations.vue'
import { useSettingsAddress } from './address'
import { useIntegrationSettings } from './integrations'

const address = useSettingsAddress()
const settings = useIntegrationSettings()

onMounted(() => void settings.read())
// A token is shown once: leaving the section forgets it.
onBeforeUnmount(() => settings.forgetTokens())

/** Add Integration: the new integration's page opens once it exists. */
async function add(): Promise<void> {
  const id = await settings.add()
  if (id)
    await address.show('integrations', id)
}

/** The integration whose page the address opens. */
const shown = computed(() => (address.section.value === 'integrations' ? address.detail.value : null))
</script>

<template>
  <SettingsIntegrations
    :integrations="settings.integrations"
    :tokens="settings.tokens"
    :shown="shown"
    :load="settings.load"
    :overlay-store="appOverlayStore"
    :on-add="add"
    :on-change="settings.change"
    :on-replace-token="settings.replaceToken"
    @retry="settings.read()"
    @show="address.show('integrations', $event)"
  />
</template>
