<script setup lang="ts">
import { computed } from 'vue'
import type { AgentMessage } from '@demicodes/protocol'
import AgentTip from '../AgentTip.vue'
import NoticeLabel from './NoticeLabel.vue'
import ReceiptTag from './ReceiptTag.vue'
import { useMessageAgents } from '../agent-links'
import { receiptOrigin, receiptRow, useOriginRevealer } from '../agent-receipts'

/**
 * What a receipt says on its notice row (`subagents.md` § Product
 * rendering), wherever the row stands, the folded face of a run included:
 * the words, the agent's name, which leads where the receipt came from
 * and, pointed at, names the agent and what it runs on, and its tag.
 */
const props = defineProps<{ message: AgentMessage }>()

const row = computed(() => receiptRow(props.message))
const agents = useMessageAgents()
// The main agent's name says all there is; a subagent's tip says more.
const sender = computed(() => {
  const number = props.message.sender?.number
  return number === undefined ? null : (agents()?.find(number) ?? null)
})
const reveal = useOriginRevealer()
const go = computed(() => {
  const origin = receiptOrigin(props.message)
  return origin ? reveal(origin) : undefined
})
</script>

<template>
  <NoticeLabel :notice="row.notice" :title="row.title" :subject="row.subject" :go="go">
    <template v-if="row.title && sender?.run" #tip>
      <AgentTip :number="sender.number" :description="sender.description" :run="sender.run" />
    </template>
  </NoticeLabel>
  <ReceiptTag v-if="row.tag" :tag="row.tag" />
</template>
