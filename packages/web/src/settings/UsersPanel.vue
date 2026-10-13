<script setup lang="ts">
import { onMounted } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsUsers from '@demicodes/web-ui/settings/SettingsUsers.vue'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import { useUserSettings } from './users'

const users = useUserSettings()
// The list is read as the page opens, and again after each action.
onMounted(() => void users.load())
</script>

<template>
  <AsyncRegion
    :state="users.status"
    label="Loading users…"
    error="Could not load the users."
    :detail="users.failure?.message"
    :request-id="users.failure?.requestId"
    :on-retry="users.load"
  >
    <SettingsUsers v-if="users.host" :host="users.host" :overlay-store="appOverlayStore" />
  </AsyncRegion>
</template>
