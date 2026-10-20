import { computed, nextTick, watch } from 'vue'
import type { MessageListBlock } from './pending-steers'

/**
 * Blocks the reader authored, at every stage: delivered, queued, steering,
 * still on its way.
 */
const READER_BLOCK_TYPES: ReadonlySet<MessageListBlock['type']> = new Set([
  'user',
  'steer',
  'queued_message',
  'pending_steer',
  'pending_submission',
])

/**
 * A message the reader just sent scrolls the transcript to the bottom, even
 * when they had scrolled up to read: the list follows what they did, the way a
 * chat does. The reply that follows is tracked only while they stay at the
 * bottom, which is the scroller's own rule. Every transcript scroller applies
 * this one, so a send reads the same in the product and the gallery.
 *
 * A message is not just sent in these cases:
 *
 * - a message read from a page of history, in its light form, wherever the page
 *   puts it
 * - a message not delivered, which shows where it was sent once the window that
 *   holds that place is read, and was on its way, a row already, when it was
 *   sent
 *
 * A message is named by `keyOf` across its stages, so the server confirming a
 * message sent is not another send.
 */
export function useFollowSentMessages(
  blocks: () => readonly MessageListBlock[],
  keyOf: (block: MessageListBlock) => string,
  scrollToBottom: () => void,
): void {
  const readerKeys = computed(() => new Set(blocks()
    .filter((block) => READER_BLOCK_TYPES.has(block.type) && !('light' in block && block.light) && !undelivered(block))
    .map(keyOf)))
  watch(readerKeys, (keys, previous) => {
    for (const key of keys) {
      if (!previous.has(key)) {
        void nextTick(scrollToBottom)
        return
      }
    }
  }, { flush: 'post' })
}

/** A message on its way whose delivery failed. */
function undelivered(block: MessageListBlock): boolean {
  return block.type === 'pending_submission' && block.submission.error !== null
}
