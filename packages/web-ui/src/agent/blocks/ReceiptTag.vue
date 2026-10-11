<script setup lang="ts">
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import type { ReceiptRow } from '../agent-receipts'

/** The tag after a receipt's agent: only what went wrong. */
defineProps<{ tag: NonNullable<ReceiptRow['tag']> }>()
</script>

<template>
  <Tag v-if="tag === 'failed'" tone="danger" class="shrink-0">Failed</Tag>
  <Tag v-else class="shrink-0">Stopped</Tag>
</template>
