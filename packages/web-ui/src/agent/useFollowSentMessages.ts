import { computed, nextTick, watch } from 'vue'
import { sentMessage } from './message-delivery'
import type { MessageListBlock } from './pending-steers'

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
 *   puts it, or whole, when it arrives above the messages the list held: the
 *   page before the window, which the reader asked for by scrolling up
 * - a message not delivered, which shows where it was sent once the window that
 *   holds that place is read, and was on its way, a row already, when it was
 *   sent
 *
 * A message is any row that shows one at any stage of its delivery
 * (`sentMessage`), named by `keyOf` across its stages, so the server confirming
 * or starting a message sent is not another send.
 */
export function useFollowSentMessages(
  blocks: () => readonly MessageListBlock[],
  keyOf: (block: MessageListBlock) => string,
  scrollToBottom: () => void,
): void {
  const readerKeys = computed(() => blocks()
    .filter((block) => sentMessage(block) !== undefined && !('light' in block && block.light) && !undelivered(block))
    .map(keyOf))
  watch(readerKeys, (keys, previous) => {
    const known = new Set(previous)
    // A message that comes in above the last one the list held is read from
    // an earlier page, not sent.
    const lastKnown = keys.findLastIndex((key) => known.has(key))
    if (keys.some((key, index) => !known.has(key) && index > lastKnown)) {
      void nextTick(scrollToBottom)
    }
  }, { flush: 'post' })
}

/** A message on its way whose delivery failed. */
function undelivered(block: MessageListBlock): boolean {
  return block.type === 'pending_submission' && block.submission.error !== null
}
