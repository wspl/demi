<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import type { Action } from '../ui/operations'
import ServerUpdates from './ServerUpdates.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import { t } from './locales/text'
import type { ServerUpgrade, UpgradeChannel } from './server-upgrade'

/**
 * Settings › Updates: its title at once, and the server's updates under it
 * once the page holds their state, which a page that just loaded, as after
 * an update, waits for.
 */
defineProps<{
  /** The server's updates; null until the page holds them. */
  state: ServerUpgrade | null
  /** Where the read of the state stands while the page holds none. */
  load: 'loading' | 'ready' | 'failed'
  overlayStore: OverlayStore
  onSetChannel?: Action<[UpgradeChannel]>
  onSetPrepareAhead?: Action<[boolean]>
  onCheck?: Action
  onCountAgents?: () => Promise<number>
  onUpdate?: Action<[string]>
}>()

defineEmits<{ retry: [] }>()
</script>

<template>
  <SettingsPage :title="t('updates.title')" :description="t('updates.description')">
    <ServerUpdates
      v-if="state"
      :state="state"
      :overlay-store="overlayStore"
      :on-set-channel="onSetChannel"
      :on-set-prepare-ahead="onSetPrepareAhead"
      :on-check="onCheck"
      :on-count-agents="onCountAgents"
      :on-update="onUpdate"
    />
    <SettingsGroup v-else>
      <AsyncRegion :state="load" @retry="$emit('retry')" />
    </SettingsGroup>
  </SettingsPage>
</template>
