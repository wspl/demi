<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsCredentials from '@demicodes/web-ui/settings/SettingsCredentials.vue'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import { useProduct } from '../state/product'
import { useCredentialSettings } from './credentials'

const product = useProduct()
const credentials = useCredentialSettings()
</script>

<template>
  <AsyncRegion :state="product.listLoad('credentials')" :on-retry="() => product.retryList('credentials')">
    <SettingsCredentials
      :credentials="credentials.credentials"
      :overlay-store="appOverlayStore"
      :create-credential="credentials.createCredential"
      :delete-credential="credentials.deleteCredential"
    />
  </AsyncRegion>
</template>
