<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsUsers from '@demicodes/web-ui/settings/SettingsUsers.vue'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import { useProduct } from '../state/product'
import { useUserSettings } from './users'

const users = useUserSettings()
const product = useProduct()
</script>

<template>
  <!-- The list is the channel's `users` part, loading until the first state arrives. -->
  <AsyncRegion :state="product.load" label="Loading users…" error="Could not load the users." :on-retry="product.reconnect">
    <SettingsUsers v-if="users.host" :host="users.host" :overlay-store="appOverlayStore" />
  </AsyncRegion>
</template>
