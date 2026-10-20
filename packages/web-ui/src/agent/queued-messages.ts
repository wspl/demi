import type { AutomationMark, MessageComment, QueuedMessage, UserContentBlock } from '@demicodes/protocol'

export interface QueueDividerBlock {
  type: 'queue_divider'
  id: 'queue-divider'
  count: number
}

export interface QueuedRenderBlock {
  type: 'queued_message'
  id: string
  queueId: string
  content: UserContentBlock[]
  /** The comments it carries; absent with none. */
  comments?: MessageComment[]
  /** The automation's run that queued it; none for the user's own. */
  automation?: AutomationMark
}

/**
 * The queue's messages as the list shows them, under its divider: a message row
 * for each entry, drawn from its content.
 */
export function queuedMessagesToRenderBlocks(queue: readonly QueuedMessage[]): QueuedRenderBlock[] {
  return queue.map((item) => ({
    type: 'queued_message' as const,
    id: `queued:${item.id}`,
    queueId: item.id,
    content: item.content,
    comments: item.comments,
    automation: item.automation,
  }))
}
