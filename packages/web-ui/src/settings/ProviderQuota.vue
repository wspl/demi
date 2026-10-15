<script setup lang="ts">
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import RequestIdLine from '../ui/RequestIdLine.vue'
import { computed, useTemplateRef, watch } from 'vue'
import { useElementVisibility } from '@vueuse/core'
import QuotaWindowRow from './QuotaWindowRow.vue'
import type { SettingsQuotaWindow } from './types'

/**
 * Displays an account's quota, each window a `QuotaWindowRow`, and requests a
 * free refresh each time it becomes visible. A failed manual refresh is a fact
 * of the account, so its line, the vendor's words in the secondary color,
 * stands above the windows wherever they show (`usage-and-quota.md`
 * § Refreshing in the product), until the account's next answer.
 */
const props = defineProps<{
  windows: SettingsQuotaWindow[]
  autoRefresh: boolean
  /** The vendor's words for the last manual refresh that failed; none after the next answer. */
  error?: ShownError | null
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
    <div v-if="error" class="mb-2 text-fg-subtle">
      <p class="break-words">{{ error.message }}</p>
      <RequestIdLine v-if="error.requestId" :request-id="error.requestId" />
    </div>
    <!-- Empty snapshots still have a visible region, so the first display can fetch them. -->
    <span v-if="!windows.length">{{ t('quota.unavailable') }}</span>
    <div v-else class="flex flex-col gap-2.5">
      <QuotaWindowRow v-for="window in windows" :key="window.id" :window="window" />
    </div>
  </div>
</template>
