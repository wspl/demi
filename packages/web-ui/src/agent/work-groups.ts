import type { MessageListBlock } from './pending-steers'
import { upperFirst } from '@demicodes/utils'
import { parseToolCallInput } from './block-helpers'
import { commandIdText, toolRenderKind, type ToolRenderKind } from './tool-rendering'

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
 * shows only while it is the newest block of a running turn, as what the
 * agent is doing; once anything follows it or the turn ends, it has nothing
 * to show and goes. The steps at the end of a running turn are
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
    const live = running && next === undefined
    const steps = blocks.slice(index, end).filter((step, at, run): step is WorkStep =>
      isStep(step) && !(isEmptyThinking(step) && !(live && at === run.length - 1)))
    const calls = steps.filter((step) => step.type !== 'thinking').length
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

/** A command count as a row says it. */
function commands(count: number): string {
  return count === 1 ? '1 command' : `${count} commands`
}

/** A step's tool, for a call written or being written. */
function stepTool(step: WorkStep): string | null {
  switch (step.type) {
    case 'thinking':
      return null
    case 'tool_call':
      return step.toolName
    case 'pending_call':
      return step.call.toolName
  }
}

/**
 * The run's calls by tool, in the order the run first called each, with the
 * commands each touched: a `shell_exec` call starts a command of its own, a
 * `shell_status` call looks at the command its input names. A call still
 * being written, or one whose input names no command, counts as a command
 * of its own. A failed call counts as any other.
 */
function callsByTool(steps: readonly WorkStep[]): Map<string, Set<string>> {
  const tools = new Map<string, Set<string>>()
  for (const step of steps) {
    const tool = stepTool(step)
    if (tool === null)
      continue
    const named = step.type === 'tool_call' && toolRenderKind(tool) === 'shell_status'
      ? commandIdText(parseToolCallInput(step).commandId)
      : undefined
    const touched = tools.get(tool) ?? new Set<string>()
    touched.add(named === undefined ? stepKey(step) : `command:${named}`)
    tools.set(tool, touched)
  }
  return tools
}

/** How a row words each kind of call, in the past once the run ended and in the present while it runs. */
const WORDS = {
  ended: {
    shell_exec: (count: number) => `ran ${commands(count)}`,
    shell_status: (count: number) => `checked ${commands(count)}`,
    yield: () => 'waited',
    generic: (_count: number, tool: string) => `used ${tool}`,
  },
  running: {
    shell_exec: (count: number) => `running ${commands(count)}`,
    shell_status: (count: number) => `checking ${commands(count)}`,
    yield: () => 'waiting',
    generic: (_count: number, tool: string) => `using ${tool}`,
  },
} satisfies Record<string, Record<ToolRenderKind, (count: number, tool: string) => string>>

/** Each kind of call the run made, once, in the order it first made it, joined as one phrase. */
function describeCalls(steps: readonly WorkStep[], tense: keyof typeof WORDS): string {
  const parts = [...callsByTool(steps)].map(([tool, touched]) =>
    WORDS[tense][toolRenderKind(tool)](touched.size, tool))
  return upperFirst(parts.join(', '))
}

/**
 * What an open group that still runs says it is doing: its steps show under
 * it, so it stands still. A run that starts commands says how many; one
 * that only looks, waits or uses other tools says so as its ended row
 * would, in the present; a run of thinking alone is thinking.
 */
export function workRunning(steps: readonly WorkStep[]): string {
  const ran = callsByTool(steps).get('shell_exec')?.size ?? 0
  if (ran > 0)
    return `Running ${commands(ran)}`
  return describeCalls(steps, 'running') || 'Thinking'
}

/** What an ended group says it did: each kind of call once, in the order the run first made it (`runtime.md` § Work groups). */
export function workSummary(steps: readonly WorkStep[]): string {
  return describeCalls(steps, 'ended')
}
