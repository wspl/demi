<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useScroll } from '@vueuse/core'
import type { Block, QueuedMessage, SessionPhase } from '@demicodes/core'
import { useBlockVirtualizer, type PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import { getVisibleBlocks } from './visible-blocks'
import { isTextBlockStreaming, isThinkingBlockStreaming } from './block-streaming'
import { pendingSteersToRenderBlocks, type MessageListBlock } from './pending-steers'
import { queuedMessagesToRenderBlocks } from './queued-messages'
import { shouldShowTailLoading } from './tail-loading'
import type { PendingSteerMessage } from './types'
import AgentMessageVirtualBlock from './blocks/AgentMessageVirtualBlock.vue'
import LoadingBlock from './blocks/LoadingBlock.vue'
import { COMPOSER_CLEARANCE_PX } from './composer-clearance'

const props = defineProps<{
  conversationId: string
  blocks: Block[]
  pendingSteers: PendingSteerMessage[]
  queue: QueuedMessage[]
  phase: SessionPhase
  bottomOffset: number
  persistedScrollState: PersistedScrollState | undefined
}>()

const emit = defineEmits<{
  saveScrollState: [conversationId: string, state: PersistedScrollState | undefined]
  deletePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  deleteQueued: [id: string]
  sendQueued: [id: string]
}>()

const visibleTranscriptBlocks = computed(() => getVisibleBlocks(props.blocks))
const renderBlocks = computed<MessageListBlock[]>(() => [
  ...visibleTranscriptBlocks.value,
  ...pendingSteersToRenderBlocks(props.pendingSteers),
  ...queuedMessagesToRenderBlocks(props.queue),
])

const shouldShowLoading = computed(() => shouldShowTailLoading(props.phase, visibleTranscriptBlocks.value, renderBlocks.value))

// Every streamed delta re-renders the visible rows; the lookup must not rescan the transcript per row.
const transcriptIndexById = computed(() => new Map(visibleTranscriptBlocks.value.map((block, index) => [block.id, index])))

function transcriptIndexAt(index: number): number {
  const block = renderBlocks.value[index]
  return block ? transcriptIndexById.value.get(block.id) ?? -1 : -1
}

function isStreamingThinkingAt(index: number): boolean {
  return isThinkingBlockStreaming(visibleTranscriptBlocks.value, props.phase, transcriptIndexAt(index))
}

function isStreamingTextAt(index: number): boolean {
  return isTextBlockStreaming(visibleTranscriptBlocks.value, props.phase, transcriptIndexAt(index))
}

// The next block's createdAt marks when a thinking block stopped (null while it's still the last,
// i.e. actively thinking). Lets ThinkingBlock show a frozen "thought for Xs" that survives reload.
function thinkingEndedAt(index: number): string | null {
  const transcriptIndex = transcriptIndexAt(index)
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

defineExpose({
  isAtBottom,
  scrollToBottom,
})
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
        <p class="text-conversation text-fg-faint">No messages yet. Start a conversation.</p>
      </div>
      <div
        v-else
        class="w-full pt-2"
        :style="{ paddingBottom: `${props.bottomOffset + COMPOSER_CLEARANCE_PX}px` }"
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
              :is-text-streaming="isStreamingTextAt(item.index)"
              :thinking-ended-at="thinkingEndedAt(item.index)"
              @delete-pending-steer="(id) => emit('deletePendingSteer', id)"
              @interrupt-pending-steer="(id) => emit('interruptPendingSteer', id)"
              @delete-queued="(id) => emit('deleteQueued', id)"
              @send-queued="(id) => emit('sendQueued', id)"
            />
          </div>
        </div>
        <LoadingBlock v-if="shouldShowLoading" />
      </div>
    </div>
  </div>
</template>
