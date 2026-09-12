import { computed, nextTick, watch } from 'vue'
import type { MessageListBlock } from './pending-steers'

/** Blocks the reader authored, at every stage: delivered, queued, steering, still on its way. */
const READER_BLOCK_TYPES: ReadonlySet<MessageListBlock['type']> = new Set([
  'user',
  'steer',
  'queued_message',
  'pending_steer',
  'pending_submission',
])

/**
 * A message the reader just sent scrolls the transcript to the bottom, even
 * when they had scrolled up to read: the list follows what they did, the way
 * a chat does. The reply that follows is tracked only while they stay at the
 * bottom, which is the scroller's own rule. Every transcript scroller applies
 * this one, so a send reads the same in the product and the gallery.
 */
export function useFollowSentMessages(
  blocks: () => readonly MessageListBlock[],
  scrollToBottom: () => void,
): void {
  const readerBlockIds = computed(
    () => new Set(blocks().filter((block) => READER_BLOCK_TYPES.has(block.type)).map((block) => block.id)),
  )
  watch(readerBlockIds, (ids, previous) => {
    for (const id of ids) {
      if (!previous.has(id)) {
        void nextTick(scrollToBottom)
        return
      }
    }
  }, { flush: 'post' })
}
