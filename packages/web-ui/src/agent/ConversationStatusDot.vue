<script setup lang="ts">
import { computed } from 'vue'
import CornerDot from '../ui/CornerDot.vue'
import type { StatusDotTone } from '../ui/StatusDot.vue'
import type { ConversationStatus } from './conversation-status'

/** A conversation's status on its tab mark: running breathes in the accent, failure and abort are red, done green. */
const props = defineProps<{
  status: ConversationStatus
}>()

const tone = computed<StatusDotTone | null>(() => {
  if (props.status === 'active')
    return 'accent'
  if (props.status === 'error' || props.status === 'aborted')
    return 'danger'
  if (props.status === 'done')
    return 'success'
  return null
})
</script>

<template>
  <CornerDot :tone="tone" :pulse="status === 'active'" />
</template>
