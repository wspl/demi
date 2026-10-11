<script setup lang="ts">
import { computed } from 'vue'
import { Bot, MoveRight, ShieldCheck } from '@lucide/vue'
import type { AgentMessage } from '@demicodes/protocol'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import NoticeRow from './NoticeRow.vue'
import ReceiptTag from './ReceiptTag.vue'
import { receiptOrigin, receiptRow, useOriginRevealer } from '../agent-receipts'
import { provideBlockScope } from '../whole-blocks'

/**
 * Input another agent, the user's decision or Demi sent the agent, as a
 * notice row of the same form as a command's end (`subagents.md` § Product
 * rendering). It does not open: what it would show is where the agent's
 * name leads, the call that sent the update or the answer that is the
 * result, and a notice that names no agent says all there is.
 */
const props = defineProps<{
  message: AgentMessage
  /** The receipt is a lone step of the running turn, its activity, and shimmers. */
  working?: boolean
}>()
// What the row shows, the light form keeps: it never opens to read the block whole.
provideBlockScope(() => undefined)
const row = computed(() => receiptRow(props.message))
const reveal = useOriginRevealer()
const go = computed(() => {
  const origin = receiptOrigin(props.message)
  return origin ? reveal(origin) : undefined
})
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <NoticeRow :notice="row.notice" :title="row.title" :subject="row.subject" :go="go" :loading="working">
      <template #icon>
        <ShieldCheck v-if="message.event.type === 'permission'" :size="ICON_PX.in28" />
        <MoveRight v-else-if="message.event.type === 'moved' || message.event.type === 'move_failed'" :size="ICON_PX.in28" />
        <Bot v-else :size="ICON_PX.in28" />
      </template>
      <template v-if="row.tag" #tag>
        <ReceiptTag :tag="row.tag" />
      </template>
    </NoticeRow>
  </div>
</template>
