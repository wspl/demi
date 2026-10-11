<script setup lang="ts">
import { computed } from 'vue'
import { Bot, MoveRight, ShieldCheck } from '@lucide/vue'
import type { AgentMessage } from '@demicodes/protocol'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import NoticeRow from './NoticeRow.vue'
import { receiptOrigin, receiptRow, useOriginRevealer } from '../agent-receipts'

/**
 * Input another agent, the user's decision or Demi sent the agent, as a
 * notice row of the same form as a command's end (`subagents.md` § Product
 * rendering): a click on the agent's name opens where it came from. It
 * opens to the message or the result.
 */
const props = defineProps<{ message: AgentMessage }>()
const isOpen = defineModel<boolean>('open', { default: false })
const row = computed(() => receiptRow(props.message))
const reveal = useOriginRevealer()
const go = computed(() => {
  const origin = receiptOrigin(props.message)
  return origin ? reveal(origin) : undefined
})
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <NoticeRow v-model:open="isOpen" :notice="row.notice" :title="row.title" :go="go">
      <template #icon>
        <ShieldCheck v-if="message.event.type === 'permission'" :size="ICON_PX.in28" />
        <MoveRight v-else-if="message.event.type === 'moved' || message.event.type === 'move_failed'" :size="ICON_PX.in28" />
        <Bot v-else :size="ICON_PX.in28" />
      </template>
      <template v-if="row.tag" #tag>
        <Tag v-if="row.tag === 'failed'" tone="danger" class="shrink-0">Failed</Tag>
        <Tag v-else class="shrink-0">Stopped</Tag>
      </template>
      <template #body>
        <div class="px-3 py-2 text-[13px] leading-5 text-fg-subtle">
          <StreamedMarkdown :content="message.content" :streaming="false" />
        </div>
      </template>
    </NoticeRow>
  </div>
</template>
