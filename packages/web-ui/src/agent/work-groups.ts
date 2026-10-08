import type { MessageListBlock } from './pending-steers'
import type { ToolCallBlock } from './block-types'
import { toolRenderKind } from './tool-rendering'

/** A step of the agent's work: a thinking block, a tool call, or a call the model is still writing. */
export type WorkStep = Extract<MessageListBlock, { type: 'thinking' | 'tool_call' | 'pending_call' }>

/**
 * Consecutive steps the transcript shows as one row: while they run, the
 * newest step rolls over the one before; once they end, a summary that
 * opens to every step.
 */
export interface WorkGroupBlock {
  type: 'work_group'
  id: string
  steps: WorkStep[]
  /** The steps are the end of a running turn: the row shows the newest step. */
  live: boolean
  /** The start of the block after the group, when the last step, if thinking, ended. */
  endedAt: string | null
}

function isStep(block: MessageListBlock): block is WorkStep {
  return block.type === 'thinking' || block.type === 'tool_call' || block.type === 'pending_call'
}

/** What names a step across its forms: a call keeps its key from being written to its block, so its row never rolls again. */
export function stepKey(step: WorkStep): string {
  switch (step.type) {
    case 'thinking':
      return step.id
    case 'tool_call':
      return step.toolUseId
    case 'pending_call':
      return step.call.toolUseId
  }
}

function isEmptyThinking(block: MessageListBlock): boolean {
  return block.type === 'thinking' && block.text.trim() === ''
}

/**
 * The list's rows with the work grouped. A thinking block without text
 * shows only when a reply follows it, or while it is the newest block; any
 * other block after it covers it. The steps at the end of a running turn are
 * always one group, so each new step rolls over the one before in one row.
 * Once they ended, they stay one group when they hold a call and another
 * step, and a lone thinking stays the group it was; a lone call is its own
 * row.
 */
export function groupWork(blocks: readonly MessageListBlock[], running: boolean): MessageListBlock[] {
  const rows: MessageListBlock[] = []
  let index = 0
  while (index < blocks.length) {
    const block = blocks[index]!
    if (!isStep(block)) {
      rows.push(block)
      index += 1
      continue
    }
    let end = index
    while (end < blocks.length && isStep(blocks[end]!))
      end += 1
    const next = blocks[end]
    const steps = blocks.slice(index, end).filter((step, at, run): step is WorkStep =>
      isStep(step) && !(isEmptyThinking(step) && (at < run.length - 1 || (next !== undefined && next.type !== 'text'))))
    const calls = steps.filter((step) => step.type !== 'thinking').length
    const live = running && next === undefined
    const grouped = live
      ? steps.length > 0
      : (calls >= 1 && steps.length >= 2) || (steps.length === 1 && steps[0]!.type === 'thinking')
    if (grouped) {
      rows.push({
        type: 'work_group',
        id: `work:${block.id}`,
        steps,
        live,
        endedAt: next && 'createdAt' in next ? next.createdAt : null,
      })
    }
    else {
      rows.push(...steps)
    }
    index = end
  }
  return rows
}

function commandCount(steps: readonly WorkStep[]): number {
  return steps.filter((step) =>
    (step.type === 'tool_call' && toolRenderKind(step.toolName) === 'shell_exec')
    || (step.type === 'pending_call' && toolRenderKind(step.call.toolName) === 'shell_exec')).length
}

/** What an open group that still runs says it is doing: its steps show under it, so it stands still. */
export function workRunning(steps: readonly WorkStep[]): string {
  const ran = commandCount(steps)
  return ran === 1 ? 'Running 1 command' : `Running ${ran} commands`
}

/** What an ended group says it did: the commands it ran, or else how many steps it took. */
export function workSummary(steps: readonly WorkStep[]): string {
  const calls = steps.filter((step): step is ToolCallBlock => step.type === 'tool_call')
  const ran = calls.filter((call) => toolRenderKind(call.toolName) === 'shell_exec').length
  if (ran > 0)
    return ran === 1 ? 'Ran 1 command' : `Ran ${ran} commands`
  return calls.length === 1 ? '1 step' : `${calls.length} steps`
}
