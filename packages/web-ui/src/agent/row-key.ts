import type { MessageListBlock } from './pending-steers'
import { isWorkStep, workRowId } from './work-groups'

/**
 * What names a row of the list across the forms it takes
 * (`product.md` § Following the agent's work): the message being sent and
 * the message the server confirmed are one row, named by the message's id,
 * which the confirmed message keeps as its turn's id; a steer waiting for
 * the turn and the steer written, and a queued message and the message it
 * becomes, likewise. A step shown alone is named as the run it starts, so
 * it is the same row as that run's group. The list keeps a row it names the
 * same, with the height it measured, and changes it in place, rather than
 * building a new one that it lays out at a guess for a frame.
 */
export function rowKey(block: MessageListBlock): string {
  if (isWorkStep(block)) {
    return workRowId(block)
  }
  switch (block.type) {
    case 'user':
      return `message:${block.turnId}`
    case 'pending_submission':
      return `message:${block.submission.id}`
    case 'queued_message':
      return `message:${block.queueId}`
    case 'steer':
      return `message:${block.id}`
    case 'pending_steer':
      return `message:${block.pendingSteerId}`
    default:
      return block.id
  }
}
