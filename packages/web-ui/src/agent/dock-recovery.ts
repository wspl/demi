import type { Block, SessionPhase } from '@demicodes/core'
import { getVisibleBlocks } from './visible-blocks'

/**
 * The dock offers Resume when the conversation's tail did not finish: an abort or a terminal
 * provider error, with nothing running. Older aborts and errors are records, not entry points.
 */
export function canResumeFromDock(phase: SessionPhase, blocks: readonly Block[]): boolean {
  if (phase !== 'idle') return false
  const tail = getVisibleBlocks(blocks).at(-1)
  return tail?.type === 'abort' || tail?.type === 'error'
}
