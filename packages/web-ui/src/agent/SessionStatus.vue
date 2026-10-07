<script setup lang="ts">
import { computed } from 'vue'
import { MessageSquareDashed, MessageSquareOff } from '@lucide/vue'
import RegionStatus from '../ui/RegionStatus.vue'
import { sessionStatusCopy, type SessionStatusKind } from './session-status'

/**
 * The session pane's stand-in for the transcript: loading, failed, empty,
 * missing, or no conversation open. It is the shared `RegionStatus` pane over
 * the session copy table, filling the pane it replaces. Only the kinds with
 * nothing under them act: a failure retries, a missing or absent conversation
 * offers to start one. An empty conversation leaves that to its composer.
 */
const props = defineProps<{
  kind: SessionStatusKind
  /** A failed restore names the reason under the label. */
  detail?: string | null
}>()

const emit = defineEmits<{
  retry: []
  create: []
}>()

const copy = computed(() => sessionStatusCopy(props.kind))
const status = computed(() => (props.kind === 'loading' || props.kind === 'failed' ? props.kind : 'note'))
</script>

<template>
  <RegionStatus
    class="h-full min-h-0 flex-1"
    :status="status"
    :icon="kind === 'empty' || kind === 'none' ? MessageSquareDashed : kind === 'missing' ? MessageSquareOff : undefined"
    :label="copy.label"
    :loading-label="sessionStatusCopy('loading').label"
    :detail="detail"
    :action="copy.action === 'create' ? 'Start a Conversation' : undefined"
    :on-retry="copy.action === 'retry' ? () => emit('retry') : undefined"
    @action="emit('create')"
  />
</template>
