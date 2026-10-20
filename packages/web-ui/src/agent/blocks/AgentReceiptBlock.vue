<script setup lang="ts">
import type { AgentMessage } from '@demicodes/protocol'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import NoticeRow from './NoticeRow.vue'
import AgentReceiptFace from './AgentReceiptFace.vue'
import { receiptIcon } from './tool-row-icon'
import { provideBlockScope } from '../whole-blocks'

/**
 * Input another agent, the user's decision or Demi sent the agent, as a notice
 * row of the same form as a command's end.
 *
 * The row does not open. What it would show is where the agent's name leads,
 * the call that sent the update or the answer that is the result, and a notice
 * that names no agent says all there is.
 *
 * It is inspectable context and never user input, so it is no user bubble and
 * offers no pending-steer, edit or Fork control.
 */
defineProps<{
  message: AgentMessage
}>()
// What the row shows, the light form keeps: it never opens to read the block
// whole.
provideBlockScope(() => undefined)
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <NoticeRow>
      <template #icon>
        <component :is="receiptIcon(message)" :size="ICON_PX.in28" />
      </template>
      <AgentReceiptFace :message="message" />
    </NoticeRow>
  </div>
</template>
