<script setup lang="ts">
import { computed } from 'vue'
import { Bot, FolderInput, ShieldCheck } from '@lucide/vue'
import type { AgentMessage } from '@demicodes/protocol'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { homePath } from '@demicodes/web-ui/files/paths'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import FunctionalBlock from './FunctionalBlock.vue'

const props = defineProps<{ message: AgentMessage }>()
const isOpen = defineModel<boolean>('open', { default: false })
const label = computed(() => {
  const event = props.message.event
  // The user's decision on a permission request, which reached the agent as this message.
  if (event.type === 'permission') {
    return event.outcome === 'allowed'
      ? `You allowed this conversation to ${event.action}`
      : `You denied this conversation permission to ${event.action}`
  }
  // The user's move of the conversation, which the user told the agent of.
  if (event.type === 'moved') {
    return `You moved this conversation to ${event.host} · ${homePath(event.path, event.home)}`
  }
  const sender = props.message.sender
  const name = sender?.description || sender?.id
  // A finished child's outcome reads as its verb: completed, failed, aborted.
  return `${name} ${event.type === 'message' ? 'sent an update' : event.outcome}`
})
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <FunctionalBlock v-model:open="isOpen" expandable>
      <template #icon>
        <ShieldCheck v-if="message.event.type === 'permission'" :size="ICON_PX.in28" />
        <FolderInput v-else-if="message.event.type === 'moved'" :size="ICON_PX.in28" />
        <Bot v-else :size="ICON_PX.in28" />
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
