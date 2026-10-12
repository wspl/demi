<script setup lang="ts">
import type { ReceiptRow } from '../agent-receipts'
import RunEndTag from './RunEndTag.vue'

/** The tag after a receipt's agent: only what went wrong. */
defineProps<{ tag: NonNullable<ReceiptRow['tag']> }>()
</script>

<template>
  <RunEndTag :kind="tag === 'failed' ? 'failed' : 'stopped'" />
</template>
