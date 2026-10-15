import type { MessageListBlock } from './pending-steers'
import { upperFirst } from '@demicodes/utils'
import { t } from './locales/text'
import { parseToolCallInput, storedShellView } from './block-helpers'
import { reportMark, reportShown } from './command-reports'
import { runningTime } from './command-end'
import { receiptRow } from './agent-receipts'
import { shellStatusLooks, toolRenderKind } from './tool-rendering'

/**
 * A step of the agent's work (`runtime.md` § Work groups): a thinking block,
 * a tool call, a call the model is still writing, a command's notice (a
 * `wakeup` that shows), or an agent's receipt: an update or a child's end.
 * The receipts of the user's own decisions are not steps: like the user's
 * words, they end a run.
 */
export type WorkStep = Extract<MessageListBlock, { type: 'thinking' | 'tool_call' | 'pending_call' | 'wakeup' | 'agent_message' }>

/** A step that tells of work done elsewhere: a command's notice or another agent's receipt. */
export type NoticeStep = Extract<WorkStep, { type: 'wakeup' | 'agent_message' }>

/** A call: one written whole, or one the model is still writing. */
type CallStep = Extract<WorkStep, { type: 'tool_call' | 'pending_call' }>

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

/**
 * Whether a block the transcript shows is a step of the work: one the
 * backend stored in a run (`runtime.md` § Work groups), or a call the model
 * is still writing, which joins the run at the transcript's end.
 */
export function isWorkStep(block: MessageListBlock): block is WorkStep {
  return block.type === 'pending_call' || ('run' in block && block.run !== undefined)
}

/** The run a step belongs to; none for a call being written, which joins whichever run it follows. */
function runOf(step: WorkStep): string | undefined {
  return step.type === 'pending_call' ? undefined : step.run
}

export function isNoticeStep(step: WorkStep): step is NoticeStep {
  return step.type === 'wakeup' || step.type === 'agent_message'
}

/** What names a step across its forms: a call keeps its key from being written to its block, so its row never rolls again. */
export function stepKey(step: WorkStep): string {
  switch (step.type) {
    case 'tool_call':
      return step.toolUseId
    case 'pending_call':
      return step.call.toolUseId
    default:
      return step.id
  }
}

/**
 * A thinking without text. One a page holds light comes without its text
 * and tells how long it is (`web-api.md` § Light form).
 */
function isEmptyThinking(block: MessageListBlock): boolean {
  if (block.type !== 'thinking') {
    return false
  }
  return 'light' in block && block.light ? block.textLength === 0 : block.text.trim() === ''
}

/**
 * The list's rows with the work grouped (`runtime.md` § Work groups): the
 * steps of one run, as the backend stored each step's run, and the calls
 * being written after them. A thinking block without text shows only while
 * it is the newest block of a running turn, as what the agent is doing;
 * once anything follows it or the turn ends, it has nothing to show and
 * goes. The steps at the end
 * of a running turn are one group from the first step on, each new step
 * showing over the one before in the same row, so a step that joins never
 * adds a row that then merges into it. Once they ended, they stay one group
 * when they hold a call, a notice or a receipt, or are a lone step; only
 * thinking alone falls apart into its steps. A group of a lone call, notice
 * or receipt shows as that step's own row (`WorkGroupBlock`).
 *
 * A group is named after the run's first block, as `stepKey` names it, so
 * the row keeps its name while the run grows, a call being written becomes
 * its block, and the run ends: the list keeps the row rather than building
 * it anew.
 */
export function groupWork(blocks: readonly MessageListBlock[], running: boolean): MessageListBlock[] {
  const rows: MessageListBlock[] = []
  let index = 0
  while (index < blocks.length) {
    const block = blocks[index]!
    if (!isWorkStep(block)) {
      rows.push(block)
      index += 1
      continue
    }
    const run = runOf(block)
    let end = index + 1
    while (end < blocks.length) {
      const next = blocks[end]!
      if (!isWorkStep(next) || (next.type !== 'pending_call' && runOf(next) !== run))
        break
      end += 1
    }
    const next = blocks[end]
    const live = running && next === undefined
    const steps = blocks.slice(index, end).filter((step, at, run): step is WorkStep =>
      isWorkStep(step) && !(isEmptyThinking(step) && !(live && at === run.length - 1)))
    const grouped = steps.length > 0 && (live || steps.length === 1 || steps.some((step) => step.type !== 'thinking'))
    if (grouped) {
      rows.push({
        type: 'work_group',
        id: workRowId(block),
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

type ThinkingStep = Extract<WorkStep, { type: 'thinking' }>
type CallBlockStep = Extract<WorkStep, { type: 'tool_call' }>
type PendingStep = Extract<WorkStep, { type: 'pending_call' }>

/**
 * What a run's row shows (`runtime.md` § Work groups): a lone call, a lone
 * call being written, notice or receipt is that step's own row, and a lone
 * thinking the thinking's; an
 * ended run says what it did, and wears the time of a command still running
 * in it, `running` naming the `shell` calls that returned while their
 * command ran on, whose live frames say whether it still does; open while
 * it runs, it stands as the stack of what runs; folded while it runs, it is
 * its newest step, and wears what that step's own row wears after its
 * title: a `shell` call's tag, the time its command has run or how it went
 * wrong (§ Rendering boundary).
 */
export type RunFace =
  | { kind: 'call'; call: CallBlockStep }
  | { kind: 'writing'; call: PendingStep }
  | { kind: 'notice'; notice: NoticeStep }
  | { kind: 'thinking'; thinking: ThinkingStep }
  | { kind: 'done'; running: CallBlockStep[] }
  | { kind: 'stack' }
  | { kind: 'step'; step: WorkStep; command: CallBlockStep | null }

/** The face of `group`'s row, `open` or folded by the reader. */
export function runFace(group: WorkGroupBlock, open: boolean): RunFace {
  const only = group.steps.length === 1 ? group.steps[0]! : null
  if (only?.type === 'tool_call')
    return { kind: 'call', call: only }
  if (only?.type === 'pending_call')
    return { kind: 'writing', call: only }
  if (only !== null && isNoticeStep(only))
    return { kind: 'notice', notice: only }
  if (only?.type === 'thinking')
    return { kind: 'thinking', thinking: only }
  if (!group.live) {
    const running = group.steps.filter((step): step is CallBlockStep =>
      step.type === 'tool_call' && toolRenderKind(step.toolName) === 'shell'
      && (step.status === 'executing' || storedShellView(step)?.status === 'running'))
    return { kind: 'done', running }
  }
  if (open)
    return { kind: 'stack' }
  const step = group.steps.at(-1)!
  const shell = step.type === 'tool_call' && toolRenderKind(step.toolName) === 'shell'
  return { kind: 'step', step, command: shell ? step : null }
}

/** The name of the row a run starting with `first` shows as; a step of an ended run without a call, shown alone, takes its own. */
export function workRowId(first: WorkStep): string {
  return `work:${stepKey(first)}`
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

function callKind(step: CallStep): CallKind {
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
    if (step.type === 'thinking' || isNoticeStep(step))
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
    run: (count: number) => t('transcript.work.ran', { count }, count),
    check: (count: number) => t('transcript.work.checked', { count }, count),
    generic: (_count: number, tool: string) => t('transcript.work.used', { tool }),
  },
  running: {
    run: (count: number) => t('transcript.work.running', { count }, count),
    check: (count: number) => t('transcript.work.checking', { count }, count),
    generic: (_count: number, tool: string) => t('transcript.work.using', { tool }),
  },
} satisfies Record<string, Record<CallKind['kind'], (count: number, tool: string) => string>>

/** The phrases of a row joined, as the language lists clauses. */
function joinPhrases(parts: readonly string[]): string {
  return parts.reduce((joined, part) => t('transcript.work.joined', { first: joined, next: part }))
}

/** Each kind of call the run made, once, in the order it first made it, joined as one phrase. */
function describeCalls(steps: readonly WorkStep[], tense: keyof typeof WORDS): string {
  const parts = callGroups(steps).map((group) => WORDS[tense][group.kind](group.touched.size, group.tool))
  return parts.length === 0 ? '' : upperFirst(joinPhrases(parts))
}

/**
 * What a notice or a receipt told: a command's end or an agent's end, each
 * by how it ended, as its own row says it, or an agent's update.
 */
type ToldEvent =
  | { subject: 'command', ending: 'finished' | 'failed' | 'stopped' | 'stopping' | 'lost' }
  | { subject: 'agent', ending: 'finished' | 'failed' | 'stopped' }
  | { subject: 'update' }

type Told = ToldEvent & {
  /** The commands or the agents it named, each once. */
  names: Set<string>
}

/** The words of what the run was told: what it named, counted, and how it ended. */
function toldWords(told: Told): string {
  const count = told.names.size
  switch (told.subject) {
    case 'update':
      return t('transcript.work.heard', { count }, count)
    case 'command':
      return t(`transcript.work.command.${told.ending}`, { count }, count)
    case 'agent':
      return t(`transcript.work.agent.${told.ending}`, { count }, count)
  }
}

/**
 * What the run's notices and receipts told, each kind once, by how its
 * command or agent ended, in the order the run first had it, counting the
 * commands and the agents they named, each once: a run without a call reads
 * them, as nothing else says what happened (`runtime.md` § Work groups).
 */
function describeNotices(steps: readonly WorkStep[]): string {
  const told = new Map<string, Told>()
  const add = (event: ToldEvent, name: string) => {
    const key = event.subject === 'update' ? event.subject : `${event.subject} ${event.ending}`
    const entry: Told = told.get(key) ?? { ...event, names: new Set<string>() }
    entry.names.add(name)
    told.set(key, entry)
  }
  for (const step of steps) {
    if (step.type === 'wakeup') {
      for (const report of step.reports.filter(reportShown))
        add({ subject: 'command', ending: reportMark(report)?.kind ?? 'finished' }, report.commandId)
    }
    else if (step.type === 'agent_message') {
      const name = step.message.sender?.id ?? step.id
      if (step.message.event.type === 'completion')
        add({ subject: 'agent', ending: receiptRow(step.message).tag ?? 'finished' }, name)
      else
        add({ subject: 'update' }, name)
    }
  }
  const parts = [...told.values()].map(toldWords)
  return parts.length === 0 ? '' : upperFirst(joinPhrases(parts))
}

/**
 * What an open group that still runs says it is doing: its steps show under
 * it, so it stands still. A run that starts commands says how many; one
 * that only checks commands or uses other tools says so as its ended row
 * would, in the present; a run of thinking alone is thinking. Once the run
 * has lasted more than 3 seconds, `elapsedMs`, it says for how long,
 * Running 4 commands for 1m32s, as Thinking for 9s does.
 */
export function workRunning(steps: readonly WorkStep[], elapsedMs: number | null = null): string {
  const ran = callGroups(steps).find((group) => group.kind === 'run')?.touched.size ?? 0
  const what = ran > 0
    ? upperFirst(t('transcript.work.running', { count: ran }, ran))
    : describeCalls(steps, 'running') || describeNotices(steps) || t('transcript.thinking.now')
  const time = runningTime(elapsedMs)
  return time === null ? what : t('transcript.work.runningFor', { what, time })
}

/**
 * What an ended group says it did (`runtime.md` § Work groups): each kind of
 * call once, in the order the run first made it; a run without a call, what
 * its notices and receipts told.
 */
export function workSummary(steps: readonly WorkStep[]): string {
  return describeCalls(steps, 'ended') || describeNotices(steps)
}
