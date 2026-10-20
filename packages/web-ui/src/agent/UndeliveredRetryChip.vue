<script setup lang="ts">
import { computed } from 'vue'
import { RotateCw } from '@lucide/vue'
import { t as shared } from '../locales/text'
import { ICON_PX } from '../ui/icon-metrics'
import SessionDockChip from './SessionDockChip.vue'
import { offersRetry } from './send-way'
import type { PendingSubmissionState } from './types'

/**
 * The dock's Retry for the messages that were not delivered, before the turn's
 * own control: shown while one of them can be delivered by sending it again,
 * and none otherwise, where each one's Edit is the way on.
 */
const props = defineProps<{
  pendingSubmissions: readonly PendingSubmissionState[]
}>()
const emit = defineEmits<{
  /** Send the messages not delivered again, in the order they were sent. */
  retry: []
}>()
const shown = computed(() => offersRetry(props.pendingSubmissions))
</script>

<template>
  <SessionDockChip v-if="shown" data-retry-undelivered @click="emit('retry')">
    <RotateCw :size="ICON_PX.in28" />
    {{ shared('common.retry') }}
  </SessionDockChip>
</template>
