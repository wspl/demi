<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useScroll } from '@vueuse/core'
import type { Block, SessionPhase } from '@demicodes/core'
import { useBlockVirtualizer, type PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import { getVisibleBlocks } from './visible-blocks'
import { isThinkingBlockStreaming } from './thinking-streaming'
import { pendingSteersToRenderBlocks, type MessageListBlock } from './pending-steers'
import { shouldShowTailLoading } from './tail-loading'
import type { PendingSteerMessage } from './types'
import AgentMessageVirtualBlock from './blocks/AgentMessageVirtualBlock.vue'
import LoadingBlock from './blocks/LoadingBlock.vue'
import ScrollToBottomButton from '@demicodes/web-ui/ui/ScrollToBottomButton.vue'

const INPUT_AREA_PADDING = 48

const props = defineProps<{
  conversationId: string
  blocks: Block[]
  pendingSteers: PendingSteerMessage[]
  phase: SessionPhase
  bottomOffset: number
  persistedScrollState: PersistedScrollState | undefined
}>()

const emit = defineEmits<{
  saveScrollState: [conversationId: string, state: PersistedScrollState | undefined]
  continue: []
  retry: []
  deletePendingSteer: [id: string]
}>()

const visibleTranscriptBlocks = computed(() => getVisibleBlocks(props.blocks))
const renderBlocks = computed<MessageListBlock[]>(() => [
  ...visibleTranscriptBlocks.value,
  ...pendingSteersToRenderBlocks(props.pendingSteers),
])

const shouldShowLoading = computed(() => shouldShowTailLoading(props.phase, visibleTranscriptBlocks.value, renderBlocks.value))

function isStreamingThinkingAt(index: number): boolean {
  const block = renderBlocks.value[index]
  if (!block || block.type !== 'thinking') return false
  const transcriptIndex = visibleTranscriptBlocks.value.findIndex((candidate) => candidate.id === block.id)
  return isThinkingBlockStreaming(visibleTranscriptBlocks.value, props.phase, transcriptIndex)
}

// The next block's createdAt marks when a thinking block stopped (null while it's still the last,
// i.e. actively thinking). Lets ThinkingBlock show a frozen "thought for Xs" that survives reload.
function thinkingEndedAt(index: number): string | null {
  const block = renderBlocks.value[index]
  if (!block || !('createdAt' in block)) return null
  const transcriptIndex = visibleTranscriptBlocks.value.findIndex((candidate) => candidate.id === block.id)
  if (transcriptIndex < 0) return null
  const next = visibleTranscriptBlocks.value[transcriptIndex + 1]
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
      <div v-if="renderBlocks.length === 0" class="grid h-full place-items-center">
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
              :recoverable="item.index === renderBlocks.length - 1 && props.phase === 'idle'"
              @continue="emit('continue')"
              @retry="emit('retry')"
              @delete-pending-steer="(id) => emit('deletePendingSteer', id)"
            />
          </div>
        </div>
        <LoadingBlock v-if="shouldShowLoading" />
      </div>
    </div>

    <ScrollToBottomButton :visible="!isAtBottom" :bottom-offset="props.bottomOffset + 36" @click="scrollToBottom" />
  </div>
</template>
