<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useScroll } from '@vueuse/core'
import type { Block, SessionPhase } from '@demi/core'
import { useBlockVirtualizer, type PersistedScrollState } from '@demi/web-ui/composables/useBlockVirtualizer'
import { getVisibleBlocks } from './visible-blocks'
import { isThinkingBlockStreaming } from './thinking-streaming'
import AgentMessageVirtualBlock from './blocks/AgentMessageVirtualBlock.vue'
import LoadingBlock from './blocks/LoadingBlock.vue'
import ScrollToBottomButton from '@demi/web-ui/ui/ScrollToBottomButton.vue'

const INPUT_AREA_PADDING = 48

const props = defineProps<{
  conversationId: string
  blocks: Block[]
  phase: SessionPhase
  bottomOffset: number
  persistedScrollState: PersistedScrollState | undefined
}>()

const emit = defineEmits<{
  saveScrollState: [conversationId: string, state: PersistedScrollState | undefined]
  continue: []
  retry: []
}>()

const renderBlocks = computed(() => getVisibleBlocks(props.blocks))

const shouldShowLoading = computed(() => {
  if (props.phase !== 'running') return false
  const blocks = renderBlocks.value
  if (blocks.length === 0) return true
  const last = blocks[blocks.length - 1]!
  return last.type === 'user' || last.type === 'resume' || last.type === 'response' || last.type === 'compaction_boundary'
})

function isStreamingThinkingAt(index: number): boolean {
  return isThinkingBlockStreaming(renderBlocks.value, props.phase, index)
}

// The next block's createdAt marks when a thinking block stopped (null while it's still the last,
// i.e. actively thinking). Lets ThinkingBlock show a frozen "thought for Xs" that survives reload.
function thinkingEndedAt(index: number): string | null {
  const next = renderBlocks.value[index + 1]
  return next && 'createdAt' in next ? next.createdAt : null
}

const scrollContainer = ref<HTMLDivElement>()

const { virtualItems, totalSize, measureElement, scrollOffset, isAtBottom, scrollToBottom, onScroll, getPersistedState } =
  useBlockVirtualizer(scrollContainer, renderBlocks, props.persistedScrollState)

onBeforeUnmount(() => {
  emit('saveScrollState', props.conversationId, getPersistedState())
})

const { isScrolling } = useScroll(scrollContainer, { idle: 1500 })

watch(
  () => props.bottomOffset,
  () => {
    nextTick(() => {
      scrollOffset.value = scrollContainer.value?.scrollTop ?? 0
    })
  },
  { flush: 'post' },
)
</script>

<template>
  <div class="relative h-full">
    <div
      ref="scrollContainer"
      class="h-full overflow-y-auto scrollbar-hidden"
      :class="isScrolling ? 'scrollbar-active' : ''"
      style="overflow-anchor: none;"
      @scroll="onScroll"
    >
      <div v-if="props.blocks.length === 0" class="grid h-full place-items-center">
        <p class="text-sm text-fg-faint">No messages yet. Start a conversation.</p>
      </div>
      <div
        v-else
        class="w-full pt-2"
        :style="{ paddingBottom: `${props.bottomOffset + INPUT_AREA_PADDING}px` }"
      >
        <div class="relative w-full" :style="{ height: `${totalSize}px` }">
          <div
            v-for="item in virtualItems"
            :key="String(item.key)"
            :data-index="item.index"
            :ref="(el) => measureElement(el as Element)"
            class="absolute inset-x-0 top-0"
            :style="{ transform: `translateY(${item.start}px)` }"
          >
            <AgentMessageVirtualBlock
              :block="renderBlocks[item.index]!"
              :conversation-id="props.conversationId"
              :is-thinking-streaming="isStreamingThinkingAt(item.index)"
              :thinking-ended-at="thinkingEndedAt(item.index)"
              @continue="emit('continue')"
              @retry="emit('retry')"
            />
          </div>
        </div>
        <LoadingBlock v-if="shouldShowLoading" />
      </div>
    </div>

    <ScrollToBottomButton :visible="!isAtBottom" :bottom-offset="props.bottomOffset + 36" @click="scrollToBottom" />
  </div>
</template>
