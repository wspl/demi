<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsCredentials from '@demicodes/web-ui/settings/SettingsCredentials.vue'
import { confirmationHost } from '../auth/handoff'
import { useSession } from '../auth/session'
import { useCredentialSettings } from './credentials'

const credentials = useCredentialSettings()
const session = useSession()
</script>

<template>
  <SettingsCredentials
    :credentials="credentials.credentials"
    :overlay-store="appOverlayStore"
    :save-credential="credentials.saveCredential"
    :delete-credential="credentials.deleteCredential"
    :reveal-credential="credentials.revealCredential"
    :verification="confirmationHost()"
    :email="session.user?.email ?? ''"
  />
</template>
