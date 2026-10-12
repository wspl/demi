<script setup lang="ts">
import { computed, useTemplateRef, watch } from 'vue'
import { useElementVisibility } from '@vueuse/core'
import QuotaWindowRow from './QuotaWindowRow.vue'
import type { SettingsQuotaWindow } from './types'

/**
 * Displays an account's quota, each window a `QuotaWindowRow`, and requests a
 * free refresh each time it becomes visible.
 */
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
  <div ref="region" class="text-[12px] leading-4 text-fg-subtle tabular-nums">
    <!-- Empty snapshots still have a visible region, so the first display can fetch them. -->
    <span v-if="!windows.length">Usage not available yet</span>
    <div v-else class="flex flex-col gap-2.5">
      <QuotaWindowRow v-for="window in windows" :key="window.id" :window="window" />
    </div>
  </div>
</template>
