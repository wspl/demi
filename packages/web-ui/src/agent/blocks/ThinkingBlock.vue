<script setup lang="ts">
import { computed } from 'vue'
import { BrainLine } from '@mingcute/vue/brain'
import { md } from '@demi/web-ui/markdown/md'
import { t } from '@demi/web-ui/infra/i18n'
import { useThinkingDisclosure } from '@demi/web-ui/composables/useThinkingDisclosure'
import CollapsibleBlock from './CollapsibleBlock.vue'

const props = defineProps<{
  thinking: string
  isStreaming: boolean
}>()

const hasContent = computed(() => props.thinking.trim().length > 0)
const isOpen = useThinkingDisclosure({
  hasContent: () => hasContent.value,
  isStreaming: () => props.isStreaming,
})
const renderedMarkdown = computed(() => md.render(props.thinking))
</script>

<template>
  <div class="px-8">
    <CollapsibleBlock v-model:open="isOpen" :expandable="hasContent" :stick-bottom="isStreaming">
      <template #icon>
        <BrainLine :size="16" />
      </template>
      <span class="min-w-0 truncate" :class="isStreaming ? 'thinking-shimmer' : ''">{{ t('agent.block.thinking') }}</span>
      <template v-if="hasContent" #body>
        <div class="markdown-body px-3 py-1 text-[13px] leading-relaxed text-fg-muted" v-html="renderedMarkdown" />
      </template>
    </CollapsibleBlock>
  </div>
</template>

<style scoped>
/* While the model is actively thinking (this block is the last one and the turn is running) we
   shimmer the label so it reads as a live "thinking" indicator — whether or not prose has streamed
   in yet — and once thinking ends it goes static. */
.thinking-shimmer {
  background: linear-gradient(
    100deg,
    rgb(105 105 105) 0%,
    rgb(105 105 105) 38%,
    rgb(250 250 250) 50%,
    rgb(105 105 105) 62%,
    rgb(105 105 105) 100%
  );
  background-size: 200% 100%;
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: thinking-shimmer 1.6s ease-in-out infinite;
}

@keyframes thinking-shimmer {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}
</style>
