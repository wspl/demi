<script setup lang="ts">
import { computed } from 'vue'
import CornerDot from '../ui/CornerDot.vue'
import { statusTone, type ConversationStatus } from './conversation-status'

/** A conversation's status on the corner of its tab's or agent's mark, in the tone it wears everywhere (`statusTone`). */
const props = defineProps<{
  status: ConversationStatus
}>()

const tone = computed(() => statusTone(props.status))
</script>

<template>
  <CornerDot :tone="tone" :pulse="status === 'active'" />
</template>
