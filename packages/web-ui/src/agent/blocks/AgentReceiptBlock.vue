<script setup lang="ts">
import type { AgentMessage } from '@demicodes/protocol'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import NoticeRow from './NoticeRow.vue'
import AgentReceiptFace from './AgentReceiptFace.vue'
import { receiptIcon } from './tool-row-icon'
import { provideBlockScope } from '../whole-blocks'

/**
 * Input another agent, the user's decision or Demi sent the agent, as a
 * notice row of the same form as a command's end (`subagents.md` § Product
 * rendering). It does not open: what it would show is where the agent's
 * name leads, the call that sent the update or the answer that is the
 * result, and a notice that names no agent says all there is.
 */
defineProps<{
  message: AgentMessage
  /** The receipt is a lone step of the running turn, its activity, and shimmers. */
  working?: boolean
}>()
// What the row shows, the light form keeps: it never opens to read the block whole.
provideBlockScope(() => undefined)
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <NoticeRow :loading="working">
      <template #icon>
        <component :is="receiptIcon(message)" :size="ICON_PX.in28" />
      </template>
      <AgentReceiptFace :message="message" />
    </NoticeRow>
  </div>
</template>
