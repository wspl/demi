<script setup lang="ts">
import { ref } from 'vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import { provideOriginRevealer } from '@demicodes/web-ui/agent/agent-receipts'
import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { productWould } from '../product-would'

/**
 * Receipt specimens over the gallery's children, as a session holds them:
 * a click on an agent's name opens the Agents window over them on that
 * agent's tab, a finished one as Completed picks it, at the block the
 * receipt came from, as the product's session does; the main agent's name
 * says what the product would show in the main transcript.
 */
defineProps<{ agents: readonly SubagentRecord[] }>()
const emit = defineEmits<{
  abort: []
  abortAgent: [id: string]
}>()

const open = ref(false)
const activeId = ref<string | null>(null)
const reveal = ref<TranscriptReveal | null>(null)
// The main agent's transcript is not among the specimens: the product would bring the block into view there.
provideOriginRevealer(({ node, blockId }) => {
  if (node === null) {
    return blockId === null ? undefined : () => productWould('The Main Transcript Shows the Call That Sent the Message')
  }
  return () => {
    activeId.value = node
    open.value = true
    reveal.value = blockId === null ? null : { node, blockId }
  }
})
</script>

<template>
  <div class="relative">
    <slot />
    <!-- The window stands over the receipts as the product's stands over the session, at a session window's height. -->
    <div class="pointer-events-none absolute inset-x-0 top-0 h-[28rem]">
      <SubagentPanel
        v-model:open="open"
        v-model:active-id="activeId"
        :agents="agents"
        :reveal="reveal"
        @revealed="reveal = null"
        @abort="emit('abort')"
        @abort-agent="(id) => emit('abortAgent', id)"
      />
    </div>
  </div>
</template>
