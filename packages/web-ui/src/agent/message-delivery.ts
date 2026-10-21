import type { MessageListBlock } from './pending-steers'

/**
 * How far a message the user sent has come: on its way (being sent, or
 * waiting as a steer or in the queue), starting (the turn started with it and
 * has not written it yet) or written in the transcript.
 *
 * A message is one row through its whole delivery, so what names a row, what
 * it shows as a face and whether the list follows a send all ask this one
 * classification (`rowKey`, `rowFace`, `useFollowSentMessages`). A row type
 * that shows a message at a new stage of its delivery is added here, and so
 * to all of them at once.
 */
export type MessageDelivery = 'on_its_way' | 'starting' | 'written'

/** The message a row shows: the id it was sent with, and how far it has come. */
export interface SentMessage {
  id: string
  delivery: MessageDelivery
}

/** The message `row` shows; undefined for a row of another kind. */
export function sentMessage(row: MessageListBlock): SentMessage | undefined {
  switch (row.type) {
    case 'pending_submission':
      return { id: row.submission.id, delivery: 'on_its_way' }
    case 'pending_steer':
      return { id: row.pendingSteerId, delivery: 'on_its_way' }
    case 'queued_message':
      return { id: row.queueId, delivery: 'on_its_way' }
    case 'starting_message':
      return { id: row.message.id, delivery: 'starting' }
    case 'user':
      return { id: row.turnId, delivery: 'written' }
    case 'steer':
      return { id: row.id, delivery: 'written' }
    default:
      return undefined
  }
}
