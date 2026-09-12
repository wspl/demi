<script setup lang="ts">
import { computed } from 'vue'
import { Bot } from '@lucide/vue'
import type { AgentMessage } from '@demicodes/core'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import { t } from '../../infra/i18n'

const props = defineProps<{ message: AgentMessage }>()
const isOpen = defineModel<boolean>('open', { default: false })
const label = computed(() => {
  const sender = props.message.sender.description || props.message.sender.id
  const event = props.message.event
  const action = t(`agent.receipt.${event.type === 'message' ? 'update' : event.outcome}`)
  return `${sender} ${action}`
})
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <FunctionalBlock v-model:open="isOpen" expandable>
      <template #icon>
        <Bot :size="ICON_PX.in28" />
      </template>
      <span class="min-w-0 truncate">{{ label }}</span>
      <template #body>
        <div class="px-3 py-2 text-[13px] leading-5 text-fg-subtle">
          <StreamedMarkdown :content="message.content" :streaming="false" />
        </div>
      </template>
    </FunctionalBlock>
  </div>
</template>
