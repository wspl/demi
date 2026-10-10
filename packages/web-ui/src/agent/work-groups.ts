import type { MessageListBlock } from './pending-steers'
import { upperFirst } from '@demicodes/utils'
import { parseToolCallInput } from './block-helpers'
import { shellStatusLooks, toolRenderKind } from './tool-rendering'

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
 * The list's rows with the work grouped (`runtime.md` § Work groups). A
 * thinking block without text shows only while it is the newest block of a
 * running turn, as what the agent is doing; once anything follows it or the
 * turn ends, it has nothing to show and goes. A lone call is its own row,
 * running or ended, so opening it shows the call; a lone thinking is a
 * group, so the first call rolls over it in place. From two steps on, the
 * steps at the end of a running turn are one group, each new step showing
 * over the one before in one row; once they ended, they stay one group when
 * they hold a call.
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
    const grouped = steps.length === 1
      ? steps[0]!.type === 'thinking'
      : steps.length >= 2 && (live || calls >= 1)
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

/**
 * What a call did, as a group's row counts it: a `shell` call whose script
 * only runs `demi shell status` checks the commands it names; any other
 * `shell` call, or one still being written, runs a command of its own; a call
 * of another tool uses that tool.
 */
type CallKind =
  | { kind: 'run' }
  | { kind: 'check', commands: string[] }
  | { kind: 'generic', tool: string }

function callKind(step: Exclude<WorkStep, { type: 'thinking' }>): CallKind {
  const tool = step.type === 'tool_call' ? step.toolName : step.call.toolName
  if (toolRenderKind(tool) === 'generic')
    return { kind: 'generic', tool }
  const script = step.type === 'tool_call' ? parseToolCallInput(step).script : undefined
  const looked = typeof script === 'string' ? shellStatusLooks(script) : null
  return looked === null ? { kind: 'run' } : { kind: 'check', commands: looked }
}

/** A run's calls by what they did, in the order the run first did each, with the commands each kind touched. */
interface CallGroup {
  kind: CallKind['kind']
  tool: string
  touched: Set<string>
}

/**
 * The run's calls by what they did, in the order the run first did each:
 * a run counts a command per call, a check the commands its calls looked
 * at, each once, and another tool its calls. A failed call counts as any
 * other.
 */
function callGroups(steps: readonly WorkStep[]): CallGroup[] {
  const groups = new Map<string, CallGroup>()
  for (const step of steps) {
    if (step.type === 'thinking')
      continue
    const call = callKind(step)
    const key = call.kind === 'generic' ? `tool:${call.tool}` : call.kind
    const group = groups.get(key) ?? { kind: call.kind, tool: call.kind === 'generic' ? call.tool : '', touched: new Set<string>() }
    if (call.kind === 'check')
      call.commands.forEach((command) => group.touched.add(command))
    else
      group.touched.add(stepKey(step))
    groups.set(key, group)
  }
  return [...groups.values()]
}

/** How a row words each kind of call, in the past once the run ended and in the present while it runs. */
const WORDS = {
  ended: {
    run: (count: number) => `ran ${commands(count)}`,
    check: (count: number) => `checked ${commands(count)}`,
    generic: (_count: number, tool: string) => `used ${tool}`,
  },
  running: {
    run: (count: number) => `running ${commands(count)}`,
    check: (count: number) => `checking ${commands(count)}`,
    generic: (_count: number, tool: string) => `using ${tool}`,
  },
} satisfies Record<string, Record<CallKind['kind'], (count: number, tool: string) => string>>

/** Each kind of call the run made, once, in the order it first made it, joined as one phrase. */
function describeCalls(steps: readonly WorkStep[], tense: keyof typeof WORDS): string {
  const parts = callGroups(steps).map((group) => WORDS[tense][group.kind](group.touched.size, group.tool))
  return upperFirst(parts.join(', '))
}

/**
 * What an open group that still runs says it is doing: its steps show under
 * it, so it stands still. A run that starts commands says how many; one
 * that only checks commands or uses other tools says so as its ended row
 * would, in the present; a run of thinking alone is thinking.
 */
export function workRunning(steps: readonly WorkStep[]): string {
  const ran = callGroups(steps).find((group) => group.kind === 'run')?.touched.size ?? 0
  if (ran > 0)
    return `Running ${commands(ran)}`
  return describeCalls(steps, 'running') || 'Thinking'
}

/** What an ended group says it did: each kind of call once, in the order the run first made it (`runtime.md` § Work groups). */
export function workSummary(steps: readonly WorkStep[]): string {
  return describeCalls(steps, 'ended')
}
