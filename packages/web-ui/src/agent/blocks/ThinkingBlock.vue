<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { RightSmallLine as ArrowIcon } from '@mingcute/vue/right-small'
import { BrainLine } from '@mingcute/vue/brain'
import { md } from '@demi/web-ui/markdown/md'
import { t } from '@demi/web-ui/infra/i18n'
import { useThinkingDisclosure } from '@demi/web-ui/composables/useThinkingDisclosure'

const props = defineProps<{
  thinking: string
  isStreaming: boolean
}>()

const isHovered = ref(false)
const hasContent = computed(() => props.thinking.trim().length > 0)
const isOpen = useThinkingDisclosure({
  hasContent: () => hasContent.value,
  isStreaming: () => props.isStreaming,
})

const renderedMarkdown = computed(() => md.render(props.thinking))
const scrollRef = ref<HTMLElement>()
const showToggle = computed(() => hasContent.value && (isHovered.value || isOpen.value))

watch(() => props.thinking, () => {
  if (!props.isStreaming || !isOpen.value) return
  nextTick(() => {
    const el = scrollRef.value
    if (el) el.scrollTop = el.scrollHeight
  })
})
</script>

<template>
  <div class="px-8">
    <div class="thinking-block">
      <div
        class="flex items-center gap-2 py-1 text-[13px] text-fg-muted transition-colors hover:text-fg-body"
        :class="hasContent ? 'cursor-pointer select-none' : ''"
        @click="hasContent && (isOpen = !isOpen)"
        @mouseenter="isHovered = true"
        @mouseleave="isHovered = false"
      >
        <div class="flex size-4 shrink-0 items-center justify-center">
          <BrainLine :size="16" />
        </div>
        <span class="min-w-0 flex-1" :class="!hasContent && isStreaming ? 'thinking-shimmer' : ''">{{ t('agent.block.thinking') }}</span>
        <div class="flex h-5 shrink-0 items-center justify-center text-xs">
          <ArrowIcon
            v-if="showToggle"
            class="size-[18px] text-fg-faint transition-transform duration-200"
            :class="isOpen ? 'rotate-90' : ''"
          />
        </div>
      </div>
      <div v-if="hasContent" class="thinking-content" :class="isOpen ? 'is-open' : ''">
        <div class="thinking-inner">
          <div ref="scrollRef" class="max-h-48 overflow-y-auto">
            <div
              class="markdown-body mt-2 border-l-2 border-fg-ghost pl-3 text-[13px] leading-relaxed text-fg-muted"
              v-html="renderedMarkdown"
            />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.thinking-content {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 0.2s ease;
}

.thinking-content.is-open {
  grid-template-rows: 1fr;
}

.thinking-inner {
  overflow: hidden;
}

/* Claude Code redacts extended-thinking text (it only returns a signature + token
   estimate), so these blocks have no prose to show. While the model is actively
   thinking (this block is the last one and the turn is running) we shimmer the label
   so it reads as a live "thinking" indicator; once thinking ends it goes static. */
.thinking-shimmer {
  background: linear-gradient(
    90deg,
    rgba(163, 163, 163, 0.45) 0%,
    rgba(200, 200, 200, 0.7) 40%,
    rgba(225, 225, 225, 0.85) 50%,
    rgba(200, 200, 200, 0.7) 60%,
    rgba(163, 163, 163, 0.45) 100%
  );
  background-size: 200% 100%;
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: thinking-shimmer 2s ease-in-out infinite;
}

@keyframes thinking-shimmer {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}
</style>
