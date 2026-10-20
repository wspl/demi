import type { HeldBlock } from '@demicodes/conversation-client'
import type { StepsNotSharedRow } from '../agent/pending-steers'

/**
 * A run of the agent's steps a share does not hold, in its place on the
 * share's page, under an id its index gives it.
 */
export interface HeldSteps {
  type: 'steps_not_shared'
  id: string
  count: number
}

/** What a share's page holds at an index: a message or a text, or a run of steps not shared. */
export type SharedItem = HeldBlock | HeldSteps

/**
 * A window of a share as its list shows it: the blocks, and each run of
 * steps as a line after the block it follows, or before the first block for
 * a run the window starts with.
 */
export function sharedRows(items: readonly SharedItem[]): { blocks: HeldBlock[]; steps: StepsNotSharedRow[] } {
  const blocks: HeldBlock[] = []
  const steps: StepsNotSharedRow[] = []
  for (const item of items) {
    if (item.type === 'steps_not_shared') {
      steps.push({ ...item, after: blocks.at(-1)?.id ?? null })
    } else {
      blocks.push(item)
    }
  }
  return { blocks, steps }
}
