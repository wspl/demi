<script setup lang="ts">
import { computed } from 'vue'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import SettingsUsage from '@demicodes/web-ui/settings/SettingsUsage.vue'
import type { SettingsTab } from '@demicodes/web-ui/settings/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { setTheme, useTheme } from '@demicodes/web-ui/theme/appTheme'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'
import DevicesPanel from './DevicesPanel.vue'
import ProvidersPanel from './ProvidersPanel.vue'

const emit = defineEmits<{ signOut: [] }>()
const resources = useResources()
const conversations = useConversations()
const { theme } = useTheme()

const tab = computed({
  get: () => resources.settingsTab as SettingsTab,
  set: (value: SettingsTab) => {
    resources.settingsTab = value
  },
})

const usage = computed(() => ({
  conversations: conversations.items.length,
  messages: conversations.items.reduce((n, c) => n + c.blocks.filter((b) => b.type === 'user').length, 0),
  cost: '$0.00',
}))
</script>

<template>
  <SettingsDialog
    v-model:tab="tab"
    :is-open="true"
    :overlay-store="appOverlayStore"
    :account="{ name: resources.username || 'Zan', plan: 'Personal workspace' }"
    @close="resources.settingsOpen = false"
  >
    <SettingsAccount
      v-if="tab === 'Account'"
      v-model:name="resources.username"
      :overlay-store="appOverlayStore"
      :theme="theme"
      @change-theme="setTheme"
      @sign-out="emit('signOut')"
    />
    <DevicesPanel v-else-if="tab === 'Devices'" />
    <ProvidersPanel v-else-if="tab === 'Providers'" />
    <SettingsUsage v-else-if="tab === 'Usage'" :usage="usage" />
  </SettingsDialog>
</template>
