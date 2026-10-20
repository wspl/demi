import type { HeldBlock, TextArrival } from '@demicodes/conversation-client'
import type { MessageListBlock } from './pending-steers'
import type { ShownBlock } from './whole-blocks'
import { upperFirst } from '@demicodes/utils'
import { formatSpan } from '../ui/dates'
import { t } from './locales/text'
import { parseToolCallInput, storedShellView } from './block-helpers'
import { reportMark, reportShown } from './command-reports'
import { receiptRow } from './agent-receipts'
import { shellStatusLooks, toolRenderKind } from './tool-rendering'

/**
 * A step of the agent's work (`rendering.md` § Work groups): a thinking block,
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
 * The steps of a run that have ended, which the transcript shows as one row
 * (`rendering.md` § Work groups): a lone step as its own row, more as what
 * they did, which opens to every step. A step that still runs is a row of
 * its own under it.
 */
export interface WorkGroupBlock {
  type: 'work_group'
  id: string
  steps: WorkStep[]
  /**
   * When each step ended, for one whose block does not record it, as a
   * thinking: the start of the block after it in the transcript, whatever
   * runs on after it, null while nothing follows it.
   */
  ends: (string | null)[]
}

/**
 * Whether a block the transcript shows is a step of the work: one the
 * backend stored in a run (`rendering.md` § Work groups), or a call the model
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

/** Whether `step` shows the block `id`: it is that block, or thinking that joins it. */
export function stepHolds(step: WorkStep & Pick<ShownBlock, 'parts'>, id: string): boolean {
  return step.id === id || (step.parts?.some((part) => part.id === id) ?? false)
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
 * and tells how long it is.
 */
function isEmptyThinking(block: MessageListBlock): boolean {
  if (block.type !== 'thinking') {
    return false
  }
  return 'light' in block && block.light ? block.textLength === 0 : block.text.trim() === ''
}

/**
 * The list's rows with the work grouped (`rendering.md` § Work groups): the
 * steps of one run, as the backend stored each step's run, and the calls
 * being written after them. A thinking block without text shows only while
 * it is the newest block of a running turn, as what the agent is doing;
 * once anything follows it or the turn ends, it has nothing to show and
 * goes. Thinking blocks next to each other are one step.
 *
 * Only a step that has ended joins its run's row: a call once it returned,
 * thinking once it is written. A step that still runs is a row of its own
 * after the run's, as the step's own block: a call waiting for its result,
 * wherever its run stands, and at the end of a running turn a call being
 * written or thinking being written. A run that holds only thinking shows
 * no row while a step of it runs, until a step that says something joins
 * it. A run's row is named after the run the backend stored (`runRowId`),
 * so it stays one row whichever of its steps ends first, as parallel calls
 * return in any order; the row a lone step ran in becomes the run's row by
 * taking its place (`succeedKeys`).
 */
export function groupWork(blocks: readonly MessageListBlock[], running: boolean): MessageListBlock[] {
  return groupRuns(blocks, running, new Map(), new Map())
}

/**
 * `groupWork` for a transcript that changes a little at a time, as a turn
 * that streams changes its last blocks many times a second: each call
 * groups the blocks it is given, and a run whose blocks, the block after
 * it and whether it is live are the ones the last call grouped keeps the
 * rows it had then, the same objects. A change at the end then regroups
 * only the run it is in, and the list redraws none of the rows above it.
 * A block that changes is replaced, never changed in place
 * (`applyTranscriptPatches`), so the same block is the same content.
 */
export function workGrouping(): (blocks: readonly MessageListBlock[], running: boolean) => MessageListBlock[] {
  let last = new Map<MessageListBlock, GroupedRun>()
  return (blocks, running) => {
    const next = new Map<MessageListBlock, GroupedRun>()
    const rows = groupRuns(blocks, running, last, next)
    last = next
    return rows
  }
}

/** The rows a run of steps was grouped into, and what they were grouped from. */
interface GroupedRun {
  blocks: readonly MessageListBlock[]
  /** The block after the run, whose start is when its last step ended; none at the transcript's end. */
  next: MessageListBlock | undefined
  live: boolean
  rows: readonly MessageListBlock[]
}

/**
 * `blocks` grouped, each run's rows taken from `last`, by the run's first
 * block, when it was grouped from the same blocks, and kept in `kept`.
 */
function groupRuns(
  blocks: readonly MessageListBlock[],
  running: boolean,
  last: ReadonlyMap<MessageListBlock, GroupedRun>,
  kept: Map<MessageListBlock, GroupedRun>,
): MessageListBlock[] {
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
    const before = last.get(block)
    const grouped = before && sameRun(before, blocks, index, end, next, live)
      ? before
      : { blocks: blocks.slice(index, end), next, live, rows: runRows(blocks.slice(index, end), next, live) }
    kept.set(block, grouped)
    rows.push(...grouped.rows)
    index = end
  }
  return rows
}

/** Whether `grouped` was grouped from the run `blocks[index..end]`, followed by `next`. */
function sameRun(
  grouped: GroupedRun,
  blocks: readonly MessageListBlock[],
  index: number,
  end: number,
  next: MessageListBlock | undefined,
  live: boolean,
): boolean {
  if (grouped.next !== next || grouped.live !== live || grouped.blocks.length !== end - index)
    return false
  return grouped.blocks.every((block, offset) => block === blocks[index + offset])
}

/**
 * The rows of one run, `blocks`, followed by `next`: the row of the steps
 * that ended, then each step that still runs.
 */
function runRows(blocks: readonly MessageListBlock[], next: MessageListBlock | undefined, live: boolean): MessageListBlock[] {
  const rows: MessageListBlock[] = []
  const steps = joinThinking(blocks.filter((step, at, run): step is WorkStep =>
    isWorkStep(step) && !(isEmptyThinking(step) && !(live && at === run.length - 1))))
  const runs = (step: WorkStep, at: number) => stepRuns(step, live, at === steps.length - 1)
  const ended = steps.filter((step, at) => !runs(step, at))
  const runningSteps = steps.filter(runs)
  // Thinking alone is one step once joined, so every ended run that shows anything is a row.
  if (ended.length > 0 && !((live || runningSteps.length > 0) && ended.every((step) => step.type === 'thinking'))) {
    // Where each block stands in the run, so each step finds the block after it at once.
    const positions = new Map(blocks.map((block, at) => [block.id, at]))
    rows.push({
      type: 'work_group',
      id: runRowId(runOf(ended[0]!) ?? stepKey(ended[0]!)),
      steps: ended,
      ends: ended.map((step) => startOf(blocks[positions.get(followedBlockId(step))! + 1] ?? next)),
    })
  }
  rows.push(...runningSteps)
  return rows
}

/**
 * The transcript block whose state `row` follows, as whether it streams and
 * when it ended: a thinking step joined of blocks next to each other
 * follows its last part, which streams while the step does and ends where
 * the step ends; any other row its own block.
 */
export function followedBlockId(row: MessageListBlock): string {
  return row.type === 'thinking' && 'parts' in row && Array.isArray(row.parts) ? row.parts.at(-1)!.id : row.id
}

/** When `block`, the block after a step, started, which is when the step, if thinking, ended. */
function startOf(block: MessageListBlock | undefined): string | null {
  return block && 'createdAt' in block ? block.createdAt : null
}

/**
 * Whether `step` still runs: a call waiting for its result, wherever it
 * stands, and in a run at the end of a running turn, `live`, a call being
 * written or thinking, `newest`, that is being written.
 */
function stepRuns(step: WorkStep, live: boolean, newest: boolean): boolean {
  switch (step.type) {
    case 'tool_call':
      return step.status === 'executing'
    case 'pending_call':
      return live
    case 'thinking':
      return live && newest
    default:
      return false
  }
}

type ThinkingStep = Extract<WorkStep, { type: 'thinking' }>
type HeldThinking = Extract<HeldBlock, { type: 'thinking' }>

/**
 * Thinking blocks next to each other, with no other step between them, as
 * the one step the page shows them as (`rendering.md` § Work groups): named
 * and started as the first, so its row stays the row it was when the next
 * one streams in and it ends where the last ends, which makes its time
 * theirs together; its text theirs in order, a paragraph apart; light while
 * any of its `parts` is, which an opened row reads whole. Each stays its
 * own block in the transcript.
 */
function joinThinking(steps: readonly WorkStep[]): WorkStep[] {
  const joined: WorkStep[] = []
  let parts: HeldThinking[] = []
  const flush = () => {
    if (parts.length === 1)
      joined.push(parts[0]!)
    else if (parts.length > 1)
      joined.push(joinedThinking(parts))
    parts = []
  }
  for (const step of steps) {
    if (step.type === 'thinking') {
      parts.push(step)
      continue
    }
    flush()
    joined.push(step)
  }
  flush()
  return joined
}

/** The step `parts`, thinking blocks next to each other, show as. */
function joinedThinking(parts: readonly HeldThinking[]): HeldThinking & { parts: readonly HeldThinking[] } {
  let text = ''
  // Each piece of a part's text arrived where it now stands in the joined
  // text; what was there before the part, the separator included, had
  // arrived already, as the text of a part read whole has.
  const arrivals: TextArrival[] = []
  for (const part of parts) {
    if (part.text.trim() === '')
      continue
    if (text !== '')
      text += '\n\n'
    const offset = text.length
    arrivals.push({ end: offset, at: Number.NEGATIVE_INFINITY })
    for (const arrival of part.arrivals ?? [])
      arrivals.push({ end: offset + arrival.end, at: arrival.at })
    text += part.text
  }
  const { light: _light, arrivals: _arrivals, ...first } = parts[0]!
  return {
    ...first,
    text,
    textLength: parts.reduce((sum, part) => sum + (part.textLength ?? 0), 0),
    ...(parts.some((part) => part.light) ? { light: true } : {}),
    ...(parts.some((part) => part.arrivals) ? { arrivals } : {}),
    parts,
  }
}
type CallBlockStep = Extract<WorkStep, { type: 'tool_call' }>

/**
 * The `shell` calls of a run that returned while their command ran on, as
 * their stored view says, whose live frames say whether it still does: the
 * run's row wears their time until they end (`rendering.md` § Work groups).
 */
export function runningOn(steps: readonly WorkStep[]): CallBlockStep[] {
  return steps.filter((step): step is CallBlockStep =>
    step.type === 'tool_call' && toolRenderKind(step.toolName) === 'shell'
    && (step.status === 'executing' || storedShellView(step)?.status === 'running'))
}

/**
 * The time `group`'s steps took, each from its start to its end, added up
 * (`rendering.md` § Work groups): a call until it returned, so a command left
 * running in the background counts only until then, and a thinking until
 * the next block started. The time the turn waited for the model between
 * steps is no step's. A notice or a receipt takes none.
 */
export function runTime(group: WorkGroupBlock): number {
  let total = 0
  group.steps.forEach((step, index) => {
    const end = step.type === 'tool_call' ? step.endedAt : step.type === 'thinking' ? group.ends[index]! : null
    if (end !== null && 'createdAt' in step)
      total += Math.max(0, Date.parse(end) - Date.parse(step.createdAt))
  })
  return total
}

/**
 * What a run of more than one step says (`rendering.md` § Work groups): what
 * it did, then its total time in the same sentence, Ran 3 commands for
 * 1m 32s, as Thought for 5s says it; a run under a second says no time, as
 * Thought briefly names none.
 */
export function runLabel(group: WorkGroupBlock): string {
  const what = workSummary(group.steps)
  const time = runTime(group)
  return time < 1000 ? what : t('transcript.work.took', { what, span: formatSpan(time) })
}

/** The name of the row of the run `run`, the id of its first block, as the backend stored it. */
export function runRowId(run: string): string {
  return `run:${run}`
}

/** The name of the row `first` shows in as a step of its own: while it runs, and as an ended run's lone step before any other step. */
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

/** How a row words each kind of call the run made. */
const WORDS = {
  run: (count: number) => t('transcript.work.ran', { count }, count),
  check: (count: number) => t('transcript.work.checked', { count }, count),
  generic: (_count: number, tool: string) => t('transcript.work.used', { tool }),
} satisfies Record<CallKind['kind'], (count: number, tool: string) => string>

/** The phrases of a row joined, as the language lists clauses. */
function joinPhrases(parts: readonly string[]): string {
  return parts.reduce((joined, part) => t('transcript.work.joined', { first: joined, next: part }))
}

/** Each kind of call the run made, once, in the order it first made it, joined as one phrase. */
function describeCalls(steps: readonly WorkStep[]): string {
  const parts = callGroups(steps).map((group) => WORDS[group.kind](group.touched.size, group.tool))
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
 * them, as nothing else says what happened (`rendering.md` § Work groups).
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
 * What an ended group says it did (`rendering.md` § Work groups): each kind of
 * call once, in the order the run first made it; a run without a call, what
 * its notices and receipts told.
 */
export function workSummary(steps: readonly WorkStep[]): string {
  return describeCalls(steps) || describeNotices(steps)
}
