<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
import ThinkingFace from './ThinkingFace.vue'
import ThinkingText from './ThinkingText.vue'
import { THINKING_ICON } from './tool-row-icon'

const props = defineProps<{
  thinking: string
  isStreaming: boolean
  createdAt: string
  /**
   * Start of the block after this one — the moment thinking ended. Null while
   * still thinking.
   */
  endedAt?: string | null
}>()

const hasContent = computed(() => props.thinking.trim().length > 0)
const isOpen = defineModel<boolean>('open', { default: false })
const rollKey = computed(() => (props.isStreaming ? 'live' : 'done'))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <FunctionalBlock
      v-model:open="isOpen"
      :expandable="hasContent"
      :stick-bottom="isStreaming"
      :roll-key="rollKey"
      :loading="isStreaming"
    >
      <template #icon>
        <component :is="THINKING_ICON" :size="ICON_PX.in28" />
      </template>
      <ThinkingFace :streaming="isStreaming" :created-at="createdAt" :ended-at="endedAt" />
      <template v-if="hasContent" #body>
        <ThinkingText :content="thinking" :streaming="isStreaming" />
      </template>
    </FunctionalBlock>
  </div>
</template>
