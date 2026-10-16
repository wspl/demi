import type { Block, ShellViewStatus } from '@demicodes/protocol'
import type { ClientSessionEvent } from '../transport/protocol'
import type { ConversationStatus } from './conversation-status'
import type { ToolCallBlock } from './block-types'
import type { CommandEnd } from './command-end'
import { storedShellView } from './block-helpers'
import { latestBlocks } from './history'
import { computed, toRaw, type ComputedRef } from 'vue'
import { t } from './locales/text'
import type { HeldTranscript } from '@demicodes/conversation-client'

/** Where a command is, as its frames and stored views say: running, exited, stopped (`aborted`), or lost. */
export type TerminalPhase = ShellViewStatus

/**
 * A command as the page shows it: from the live frames of the
 * conversation's tree (`shell-tool.md` § Live output), or, for one the page has
 * seen no frame of, from the transcript's stored view.
 */
export interface TerminalRecord {
  id: string
  /** Its tab's title: the `shell` call's title, as its row shows it, or its command id when no call is known. */
  title: string
  /** The `shell` call's script, which the terminal opens with; absent when no call is known. */
  script?: string
  phase: TerminalPhase
  startedAt: string
  endedAt?: string
  /** Its output as the page shows it, at most `LIVE_OUTPUT_CHARS` characters. */
  output: string
  /**
   * The live view's count of characters at the end of `output`; absent for
   * output from the transcript.
   */
  chars?: number
  /** Its exit code, once it exited and a live frame or a stored view said so. */
  exitCode?: number
  /** Why its runner lost it, once a live frame said so. */
  lostReason?: string
  /**
   * While it runs: a stop waits for its runner to say that it ended
   * (`runner.md` § Cancellation and completion), as a live frame said.
   */
  stopping?: boolean
  /** The `shell` call that started it, known from its live frames. */
  toolUseId?: string
  /** The subagent that runs it; absent for the root's commands. */
  subagentId?: string
}

/** The most of a command's output a page keeps: as many characters as a call's stored view holds. */
export const LIVE_OUTPUT_CHARS = 32_768

/**
 * What a page adds to a command's output that it shows up to the `shown`-th
 * character of the live view, given the view's `tail`, which ends at its
 * `chars`-th: only the characters beyond those shown, or, after a gap or
 * with nothing shown yet, the tail anew (`rendering.md` § Rendering boundary).
 * Characters are Unicode code points, as the backend counts them.
 */
export function liveOutputDelta(
  shown: number | undefined,
  tail: string,
  chars: number,
): { anew: boolean; text: string } {
  const characters = Array.from(tail)
  const added = shown === undefined ? -1 : chars - shown
  if (added < 0 || added > characters.length) {
    return { anew: true, text: tail }
  }
  return { anew: false, text: characters.slice(characters.length - added).join('') }
}

/** What a terminal shows so far: the output written, and the live view's count at its end. */
export interface TerminalShown {
  output: string
  chars: number | undefined
}

/**
 * What a terminal writes to show `output` after what it showed (`shown`,
 * null before anything): for a live view, the characters beyond those shown,
 * by the view's count (`rendering.md` § Rendering boundary); for whole output,
 * what continues it. Otherwise it starts anew, clearing what it showed when
 * it showed anything. The `prompt` line opens the terminal each time it
 * starts and is never part of the output or its count.
 */
export function terminalWrite(
  shown: TerminalShown | null,
  output: string,
  chars: number | undefined,
  prompt: string,
): { reset: boolean; text: string } {
  const delta = shown === null
    ? { anew: true, text: output }
    : chars === undefined
      ? output.startsWith(shown.output)
        ? { anew: false, text: output.slice(shown.output.length) }
        : { anew: true, text: output }
      : liveOutputDelta(shown.chars, output, chars)
  return delta.anew
    ? { reset: shown !== null, text: prompt + delta.text }
    : { reset: false, text: delta.text }
}

/**
 * The colors of a prompt line, as the escape sequences that set them
 * (`terminalPromptColors`): the prompt's (`$`, `>`), and the script's where
 * no highlighted run colors it.
 */
export interface PromptColors {
  prompt: string
  script: string
}

/** A highlighted run of a script, by where it starts and ends in it, as the escape sequences that draw it (`terminalCodeRuns`). */
export interface PromptRun {
  from: number
  to: number
  ansi: string
}

const RESET = '\x1b[0m'

/**
 * The line a command's terminal opens with, as a terminal shows what was
 * typed (`rendering.md` § Rendering boundary): `$ ` and the script's first
 * line, `> ` before each further line, the prompt muted and the script
 * highlighted as Bash by `runs`, in its own color where no run colors it;
 * the terminal wraps what is longer than its width. No script, no line.
 */
export function promptLine(script: string | undefined, colors: PromptColors, runs: readonly PromptRun[] = []): string {
  const text = script?.replace(/(\r?\n)+$/, '')
  if (!text) {
    return ''
  }
  let lines = ''
  let start = 0
  for (const [index, line] of text.split(/\r?\n/).entries()) {
    // Where the line starts in the script, past the break before it.
    start = index === 0 ? 0 : text.indexOf('\n', start) + 1
    let drawn = ''
    let at = start
    const end = start + line.length
    for (const run of runs) {
      if (run.to <= at || run.from >= end)
        continue
      if (run.from > at)
        drawn += `${colors.script}${text.slice(at, run.from)}`
      const to = Math.min(run.to, end)
      drawn += `${RESET}${run.ansi}${text.slice(Math.max(run.from, at), to)}${RESET}`
      at = to
    }
    if (at < end)
      drawn += `${colors.script}${text.slice(at, end)}`
    lines += `${colors.prompt}${index === 0 ? '$' : '>'}${RESET} ${drawn}${RESET}\n`
  }
  return lines
}

/**
 * A command's output after a live view of it: the `output` shown so far,
 * which ends at the view's `shown`-th character, with what the view adds,
 * or the view's tail anew after a gap; the last `LIVE_OUTPUT_CHARS`
 * characters of it.
 */
export function followLiveOutput(
  output: string,
  shown: number | undefined,
  tail: string,
  chars: number,
): string {
  const delta = liveOutputDelta(shown, tail, chars)
  const next = delta.anew ? delta.text : output + delta.text
  const characters = Array.from(next)
  return characters.length > LIVE_OUTPUT_CHARS
    ? characters.slice(-LIVE_OUTPUT_CHARS).join('')
    : next
}

/** A command's live view, as a `shell_output` frame carries it (`shell-tool.md` § Live output). */
export type CommandFrame = Extract<ClientSessionEvent, { type: 'shell_output' }>['status']

/**
 * The command a page shows after its frame `status` at `now`, given the
 * record it showed before, if any, and the `shell` call that started it,
 * when the transcript has it. Its start is the server's, how long it has
 * run as the frame says, so a page that attached late, after a reload or
 * in a new tab, times it from when it truly started; its end is that start
 * and how long its last frame says it ran.
 */
export function followFrame(
  current: TerminalRecord | undefined,
  status: CommandFrame,
  call: { title: string; script?: string } | undefined,
  subagentId: string | undefined,
  now: number,
): TerminalRecord {
  const startedAt = current?.startedAt ?? new Date(now - status.runningMs).toISOString()
  return {
    id: status.commandId,
    title: call?.title ?? status.commandId,
    script: call?.script,
    phase: status.status,
    startedAt,
    ...(status.status !== 'running'
      ? { endedAt: current?.endedAt ?? new Date(Date.parse(startedAt) + status.runningMs).toISOString() }
      : {}),
    output: followLiveOutput(current?.output ?? '', current?.chars, status.tail, status.chars),
    chars: status.chars,
    // How it ended, which its row marks also when its call returned before.
    exitCode: status.status === 'exited' ? status.exitCode : undefined,
    lostReason: status.status === 'lost' ? status.reason : undefined,
    stopping: status.status === 'running' && status.stopping ? true : undefined,
    toolUseId: status.toolUseId,
    ...(subagentId === undefined ? {} : { subagentId }),
  }
}

/**
 * The command each `shell` call started, by the subagent that runs it,
 * or the root for none, and the call's id, as `terminals` holds them when
 * the page has seen their live frames. A model may reuse a call's id; the
 * latest command is the one that runs. The lookup reads an index the list
 * keeps (one pass whenever a command or its call changes), so a row that
 * looks up its command reads neither the list nor any other command, and a
 * frame that changes one command's output re-reads nothing.
 */
export function useCallTerminals(
  terminals: () => readonly TerminalRecord[],
): (subagentId: string | undefined, toolUseId: string) => TerminalRecord | undefined {
  const byCall = computed(() => {
    const index = new Map<string | undefined, Map<string, TerminalRecord>>()
    for (const terminal of terminals()) {
      if (terminal.toolUseId === undefined) {
        continue
      }
      let calls = index.get(terminal.subagentId)
      if (calls === undefined) {
        calls = new Map()
        index.set(terminal.subagentId, calls)
      }
      calls.set(terminal.toolUseId, terminal)
    }
    return index
  })
  return (subagentId, toolUseId) => byCall.value.get(subagentId)?.get(toolUseId)
}

/** What the dock reads: the commands, and the transcripts of the root and its subagents that run them. */
export interface DockSource {
  terminals: readonly TerminalRecord[]
  history: HeldTranscript
  subagents: readonly { id: string; history: HeldTranscript }[]
}

/**
 * The ids of the calls in `blocks` that still run, in one pass: a model may
 * reuse a call's id, and the latest call of an id says whether it runs.
 */
function executingCalls(blocks: readonly Block[]): ReadonlySet<string> {
  const executing = new Set<string>()
  for (const block of blocks) {
    if (block.type !== 'tool_call') {
      continue
    }
    if (block.status === 'executing') {
      executing.add(block.toolUseId)
    } else {
      executing.delete(block.toolUseId)
    }
  }
  return executing
}

const NO_CALLS: ReadonlySet<string> = new Set()

/**
 * The commands the dock holds (`rendering.md` § Rendering boundary): all but
 * those whose `shell` call still runs, whose output shows under the call.
 * Each agent's running calls are a computed of their own, over the latest
 * blocks of that agent's transcript: a patch to one transcript, as each
 * streamed piece is, scans that transcript once and no other, and the
 * commands are then read once, each by the set of its agent.
 */
export function useDockTerminals(source: () => DockSource): ComputedRef<TerminalRecord[]> {
  const rootCalls = computed(() => executingCalls(latestBlocks(source().history)))
  // By the subagent's record, which the store keeps while the child is
  // listed; a record that goes takes its computed with it.
  const childCalls = new WeakMap<object, ComputedRef<ReadonlySet<string>>>()
  const callsOf = (agent: DockSource['subagents'][number]): ReadonlySet<string> => {
    const key = toRaw(agent)
    let calls = childCalls.get(key)
    if (calls === undefined) {
      calls = computed(() => executingCalls(latestBlocks(agent.history)))
      childCalls.set(key, calls)
    }
    return calls.value
  }
  const agents = computed(() => new Map(source().subagents.map((agent) => [agent.id, agent])))
  return computed(() => source().terminals.filter((terminal) => {
    if (terminal.toolUseId === undefined) {
      return true
    }
    const agent = terminal.subagentId === undefined ? undefined : agents.value.get(terminal.subagentId)
    const executing = terminal.subagentId === undefined
      ? rootCalls.value
      : agent === undefined ? NO_CALLS : callsOf(agent)
    return !executing.has(terminal.toolUseId)
  }))
}

export function isTerminalRunning(phase: TerminalPhase): boolean {
  return phase === 'running'
}

/**
 * Whether a `shell` row shimmers (`rendering.md` § Rendering boundary):
 * while its call runs, and after the call returned for as long as the
 * command it started runs, as the command's live frames say, until its end
 * arrives.
 */
export function shellRowRunning(
  callStatus: ToolCallBlock['status'],
  command: TerminalRecord | undefined,
): boolean {
  return callStatus === 'executing' || (command !== undefined && isTerminalRunning(command.phase))
}

/**
 * How a `shell` row's command ended (`rendering.md` § Rendering
 * boundary): that a stop waits for its end, while its runner has not said
 * so; none while its call runs otherwise; after, the end the command's
 * record has, from its live frames or a stored view, and otherwise the view
 * the call stored, unless that view saw the command still running, which
 * says nothing of its end.
 */
export function shellRowEnd(call: ToolCallBlock, command: TerminalRecord | undefined): CommandEnd | null {
  if (command?.stopping && isTerminalRunning(command.phase)) {
    return { status: command.phase, stopping: true }
  }
  if (call.status === 'executing') {
    return null
  }
  if (command !== undefined && !isTerminalRunning(command.phase)) {
    return {
      status: command.phase,
      ...(command.exitCode === undefined ? {} : { exitCode: command.exitCode }),
      ...(command.lostReason === undefined ? {} : { lostReason: command.lostReason }),
    }
  }
  const stored = storedShellView(call)
  return stored?.status === 'running' ? null : stored
}

/**
 * How long a `shell` row's command has run or ran (`rendering.md` § Rendering
 * boundary): while it runs, from the start the server reported for it;
 * once it ended, how long it ran, by its live frames or by the view its
 * call stored. None while that is unknown, as for a command whose call
 * returned while it ran and whose end the page never saw.
 */
export type CommandTime =
  | { kind: 'running'; since: number }
  | { kind: 'ran'; ms: number }

export function shellRowTime(call: ToolCallBlock, command: TerminalRecord | undefined): CommandTime | null {
  if (command !== undefined) {
    const since = Date.parse(command.startedAt)
    if (isTerminalRunning(command.phase))
      return { kind: 'running', since }
    if (command.endedAt !== undefined)
      return { kind: 'ran', ms: Math.max(0, Date.parse(command.endedAt) - since) }
  }
  if (call.status === 'executing') {
    return null
  }
  const stored = storedShellView(call)
  return stored && stored.status !== 'running' ? { kind: 'ran', ms: stored.runningMs } : null
}

export function runningTerminals(
  terminals: readonly TerminalRecord[],
): TerminalRecord[] {
  return terminals
    .filter((terminal) => isTerminalRunning(terminal.phase))
    .toSorted((a, b) => Date.parse(a.startedAt) - Date.parse(b.startedAt))
}

/**
 * A command's mark: running, done, or stopped; a stopped command is never
 * shown as done, and neither is a lost one, which its row tags grey as a
 * stopped one.
 */
export function terminalStatus(phase: TerminalPhase): ConversationStatus {
  if (phase === 'running') {
    return 'active'
  }
  if (phase === 'aborted' || phase === 'lost') {
    return 'aborted'
  }
  return 'done'
}

export function runningChipLabel(count: number): string {
  return t('terminals.chip', { count })
}

export function firstRunningTerminalId(
  terminals: readonly TerminalRecord[],
): string | null {
  return runningTerminals(terminals)[0]?.id ?? null
}

/** Running inspect lists every running job; an exited inspect is that job only. */
export function terminalPanelTabs(
  terminals: readonly TerminalRecord[],
  activeId: string | null,
): TerminalRecord[] {
  if (!activeId) {
    return []
  }
  const active = terminals.find((terminal) => terminal.id === activeId)
  if (!active) {
    return []
  }
  if (isTerminalRunning(active.phase)) {
    return runningTerminals(terminals)
  }
  return [active]
}
