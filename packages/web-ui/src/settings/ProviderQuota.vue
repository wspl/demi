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
  <div ref="region" class="@container text-[12px] leading-4 text-fg-subtle">
    <!-- Empty snapshots still have a visible region, so the first display can fetch them. -->
    <span v-if="!windows.length">Usage not available yet</span>
    <!-- From 20rem each window is one line: name, meter, usage, the names and usages
         aligned in columns, the meters taking the width the row gives. Narrower (a phone)
         each window puts its name and usage over a full-width meter, the usage under the
         name when both do not fit. Text wraps rather than overflows. -->
    <div
      v-else
      class="grid gap-y-1.5 text-[11px] tabular-nums @xs:grid-cols-[auto_minmax(4rem,1fr)_auto] @xs:items-center @xs:gap-x-2 @xs:gap-y-1"
    >
      <div
        v-for="window in windows"
        :key="window.id"
        class="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 @xs:contents"
      >
        <span class="wrap-anywhere">{{ window.label }}</span>
        <Meter
          class="order-last mt-0.5 @xs:order-none @xs:mt-0"
          :value="window.used"
          :max="window.max"
          :label="window.label"
        />
        <span class="ml-auto wrap-anywhere @xs:ml-0">{{ window.used }}%<template v-if="window.resets"> · resets {{ window.resets }}</template></span>
      </div>
    </div>
  </div>
</template>
