<script setup lang="ts">
import { computed } from 'vue'
import StatusDot from '@demicodes/web-ui/ui/StatusDot.vue'
import { statusTone } from '@demicodes/web-ui/agent/conversation-status'
import type { SidebarConversation } from './types'

// One quiet mark (`product.md` § Conversations and projects): yellow while a permission
// request waits for the user, the same size as every other dot and with nothing around it, over every other mark and whether the row is open or
// read; otherwise the tone the conversation's state wears on its tab too, the accent breathing
// while it runs, green once finished and red once failed, read or not. A turn the user stopped is
// their own decision, and like a conversation that ran nothing it leaves the row settled: a faint
// ring in the dot's place, so the column under a project's icon is never empty. A folded group's
// label carries the dot of a conversation in it that waits, so the mark is drawn here alone.
const props = defineProps<{
  conversation: Pick<SidebarConversation, 'status' | 'needsYou'>
}>()

const tone = computed(() => statusTone(props.conversation.status))
</script>

<template>
  <span class="flex size-3.5 shrink-0 items-center justify-center">
    <span
      v-if="conversation.needsYou"
      class="size-1.5 rounded-full bg-on-attention"
      role="img"
      aria-label="Needs your decision"
    />
    <StatusDot v-else-if="tone" :tone="tone" :class="{ 'sidebar-breath': conversation.status === 'active' }" />
    <span v-else class="size-1.5 rounded-full border border-fg-faint/60" />
  </span>
</template>

<style scoped>
.sidebar-breath {
  animation: sidebar-breath 2.4s ease-in-out infinite;
}

@keyframes sidebar-breath {
  0%,
  100% {
    opacity: 0.35;
    transform: scale(0.8);
  }
  50% {
    opacity: 1;
    transform: scale(1.15);
  }
}
</style>
