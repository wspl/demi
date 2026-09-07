<script setup lang="ts">
import Button from '../ui/Button.vue'
import Meter from '../ui/Meter.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsUsageInfo } from './types'

/** This month, across every provider: the plan and its credits, then spend and tokens. */
defineProps<{
  usage: SettingsUsageInfo
}>()

const emit = defineEmits<{
  invoices: []
  manageBilling: []
}>()
</script>

<template>
  <SettingsPage title="Usage & billing" description="This month, across every provider.">
    <SettingsGroup title="Plan">
      <SettingsRow :label="`${usage.plan} plan`" :description="`Next invoice ${usage.renews} · credits reset ${usage.resets}`">
        <Button size="sm" @click="emit('invoices')">Invoices</Button>
        <Button size="sm" @click="emit('manageBilling')">Manage billing</Button>
      </SettingsRow>
      <div class="flex flex-col gap-2 px-4 py-3">
        <div class="flex items-baseline justify-between text-[12px]">
          <span class="select-none text-fg-muted">Monthly credits</span>
          <span class="tabular-nums text-fg">{{ usage.creditsUsed.toLocaleString() }} / {{ usage.creditsMax.toLocaleString() }}</span>
        </div>
        <Meter :value="usage.creditsUsed" :max="usage.creditsMax" label="Monthly credits" />
      </div>
    </SettingsGroup>
    <SettingsGroup title="Spend by provider" description="Billed by the providers you connected with your own keys.">
      <SettingsRow v-for="row in usage.spend" :key="row.provider" :label="row.provider">
        <span class="text-chrome tabular-nums text-fg">{{ row.amount }}</span>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Tokens">
      <SettingsRow label="Input"><span class="text-chrome tabular-nums text-fg">{{ usage.input }}</span></SettingsRow>
      <SettingsRow label="Output"><span class="text-chrome tabular-nums text-fg">{{ usage.output }}</span></SettingsRow>
      <SettingsRow label="Cache reads" description="Served from prompt cache; billed at a tenth of input."><span class="text-chrome tabular-nums text-fg">{{ usage.cacheRead }}</span></SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
