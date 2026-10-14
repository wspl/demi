<script setup lang="ts">
import { computed } from 'vue'
import type { AgentMessage } from '@demicodes/protocol'
import NoticeLabel from './NoticeLabel.vue'
import ReceiptTag from './ReceiptTag.vue'
import { receiptOrigin, receiptRow, useOriginRevealer } from '../agent-receipts'

/**
 * What a receipt says on its notice row (`subagents.md` § Product
 * rendering), wherever the row stands, the folded face of a run included:
 * the words, the agent's name, which leads where the receipt came from,
 * and its tag.
 */
const props = defineProps<{ message: AgentMessage }>()

const row = computed(() => receiptRow(props.message))
const reveal = useOriginRevealer()
const go = computed(() => {
  const origin = receiptOrigin(props.message)
  return origin ? reveal(origin) : undefined
})
</script>

<template>
  <NoticeLabel :notice="row.notice" :title="row.title" :inline="row.inline" :go="go" />
  <ReceiptTag v-if="row.tag" :tag="row.tag" />
</template>
