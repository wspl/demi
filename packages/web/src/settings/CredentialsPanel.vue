<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsCredentials from '@demicodes/web-ui/settings/SettingsCredentials.vue'
import { useCredentialSettings } from './credentials'

const credentials = useCredentialSettings()
</script>

<template>
  <SettingsCredentials
    :credentials="credentials.credentials"
    :overlay-store="appOverlayStore"
    :create-credential="credentials.createCredential"
    :delete-credential="credentials.deleteCredential"
  />
</template>
