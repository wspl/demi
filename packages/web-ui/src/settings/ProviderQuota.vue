<script setup lang="ts">
import { computed, useTemplateRef, watch } from 'vue'
import { useElementVisibility } from '@vueuse/core'
import Meter from '../ui/Meter.vue'
import type { SettingsQuotaWindow } from './types'

/** Displays an account's quota and requests a free refresh each time it becomes visible. */
const props = defineProps<{
  windows: SettingsQuotaWindow[]
  autoRefresh: boolean
}>()
const emit = defineEmits<{ refresh: [] }>()
const region = useTemplateRef<HTMLDivElement>('region')
const visible = useElementVisibility(region)
watch(computed(() => visible.value && props.autoRefresh), (shown) => {
  if (shown) {
    emit('refresh')
  }
})
</script>

<template>
  <div ref="region" class="mt-1">
    <!-- Empty snapshots still have a visible region, so the first display can fetch them. -->
    <span v-if="!windows.length">Usage not available yet</span>
    <div
      v-else
      class="grid max-w-96 grid-cols-[auto_minmax(4rem,1fr)_auto] items-center gap-x-2 gap-y-1 whitespace-nowrap text-[11px] tabular-nums"
    >
      <template v-for="window in windows" :key="window.id">
        <span>{{ window.label }}</span>
        <Meter :value="window.used" :max="window.max" :label="window.label" />
        <span>{{ window.used }}%<template v-if="window.resets"> · resets {{ window.resets }}</template></span>
      </template>
    </div>
  </div>
</template>
