<script setup lang="ts">
import type { SettingsUsage } from './types'

defineProps<{
  usage: SettingsUsage
}>()
</script>

<template>
  <h3>Usage</h3>
  <div class="grid grid-cols-3 gap-3 py-6">
    <div>
      <strong class="text-[24px] font-normal">{{ usage.conversations }}</strong>
      <p class="hint">Conversations</p>
    </div>
    <div>
      <strong class="text-[24px] font-normal">{{ usage.messages }}</strong>
      <p class="hint">Messages</p>
    </div>
    <div>
      <strong class="text-[24px] font-normal">{{ usage.cost }}</strong>
      <p class="hint">Actual cost</p>
    </div>
  </div>
</template>
