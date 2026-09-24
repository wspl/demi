import type { QueuedMessage, UserContentBlock } from '@demicodes/protocol'

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
}

/** The queue as the list shows it: a message row for each entry, drawn from its content. */
export function queuedMessagesToRenderBlocks(
  queue: readonly QueuedMessage[],
): Array<QueueDividerBlock | QueuedRenderBlock> {
  if (queue.length === 0)
    return []
  return [
    { type: 'queue_divider', id: 'queue-divider', count: queue.length },
    ...queue.map((item) => ({
      type: 'queued_message' as const,
      id: `queued:${item.id}`,
      queueId: item.id,
      content: item.content,
    })),
  ]
}

export function isQueueTailBlock(block: { type: string } | undefined): boolean {
  return block?.type === 'queue_divider' || block?.type === 'queued_message'
}
