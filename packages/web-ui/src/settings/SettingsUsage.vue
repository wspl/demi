<script setup lang="ts">
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsUsage } from './types'

defineProps<{
  usage: SettingsUsage
}>()
</script>

<template>
  <SettingsPage title="Usage" description="What this workspace has used so far.">
    <SettingsGroup title="This workspace">
      <SettingsRow label="Conversations">
        <span class="text-chrome tabular-nums text-fg">{{ usage.conversations }}</span>
      </SettingsRow>
      <SettingsRow label="Messages" description="User turns across every conversation.">
        <span class="text-chrome tabular-nums text-fg">{{ usage.messages }}</span>
      </SettingsRow>
      <SettingsRow label="Actual cost" description="Billed by the providers you connected.">
        <span class="text-chrome tabular-nums text-fg">{{ usage.cost }}</span>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
