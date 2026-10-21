<script setup lang="ts">
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import RequestIdLine from '../ui/RequestIdLine.vue'
import { computed } from 'vue'
import { useRelativeTime } from '../composables/useRelativeTime'
import QuotaWindowRow from './QuotaWindowRow.vue'
import type { SettingsQuotaWindow } from './types'

/**
 * Displays an account's quota, each window a `QuotaWindowRow`, as the backend
 * last announced it (`crates/backend-providers/src/vault/quotas.rs`), with when
 * the vendor reported it under them, as a device's page says when it measured.
 * Showing it asks the vendor nothing; the Refresh usage button beside it does.
 *
 * A failed refresh is a fact of the account, so its line, the vendor's words
 * in the secondary color, stands above the windows wherever they show, until
 * the account's next answer.
 */
const props = defineProps<{
  windows: SettingsQuotaWindow[]
  /** When the vendor reported the windows, as an ISO 8601 timestamp; null or absent when unknown. */
  observedAt?: string | null
  /** The vendor's words for the last manual refresh that failed; none after the next answer. */
  error?: ShownError | null
}>()

const ago = useRelativeTime(() => props.observedAt ?? new Date().toISOString())
/** "Updated 3 minutes ago", under the windows. */
const updated = computed(() => (props.observedAt ? t('quota.updatedAt', { when: ago.value }) : null))
</script>

<template>
  <div class="text-detail leading-4 text-fg-subtle tabular-nums">
    <div v-if="error" class="mb-2 text-fg-subtle">
      <p class="break-words">{{ error.message }}</p>
      <RequestIdLine v-if="error.requestId" :request-id="error.requestId" />
    </div>
    <span v-if="!windows.length">{{ t('quota.unavailable') }}</span>
    <div v-else class="flex flex-col gap-2.5">
      <QuotaWindowRow v-for="window in windows" :key="window.id" :window="window" />
      <p v-if="updated">{{ updated }}</p>
    </div>
  </div>
</template>
