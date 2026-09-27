import type { Block } from '@demicodes/protocol'
import type { ConversationStatus } from './conversation-status'

export type TerminalPhase = 'running' | 'exited'

/**
 * A command as the page shows it: from the live frames of the
 * conversation's tree (`runtime.md` § Live output), or, for one the page has
 * seen no frame of, from the transcript's stored view.
 */
export interface TerminalRecord {
  id: string
  name: string
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
  /** The `shell_exec` call that started it, known from its live frames. */
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
 * with nothing shown yet, the tail anew (`runtime.md` § Rendering boundary).
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

/**
 * The command the `shell_exec` call `toolUseId` of the subagent
 * `subagentId`, or of the root when none, started, when the page has seen
 * its live frames. A model may reuse a call's id; the latest command is
 * the one that runs.
 */
export function callTerminal(
  terminals: readonly TerminalRecord[],
  subagentId: string | undefined,
  toolUseId: string,
): TerminalRecord | undefined {
  return terminals.findLast(
    (terminal) => terminal.toolUseId === toolUseId && terminal.subagentId === subagentId,
  )
}

/**
 * The commands the dock holds (`runtime.md` § Rendering boundary): all but
 * those whose `shell_exec` call still runs, whose output shows under the
 * call. `blocksOf` gives the transcript of the subagent that runs a
 * command, or the root's for none.
 */
export function dockTerminals(
  terminals: readonly TerminalRecord[],
  blocksOf: (subagentId: string | undefined) => readonly Block[],
): TerminalRecord[] {
  return terminals.filter((terminal) => {
    if (!terminal.toolUseId) {
      return true
    }
    const call = blocksOf(terminal.subagentId).findLast(
      (block) => block.type === 'tool_call' && block.toolUseId === terminal.toolUseId,
    )
    return call?.type !== 'tool_call' || call.status !== 'executing'
  })
}

export function isTerminalRunning(phase: TerminalPhase): boolean {
  return phase === 'running'
}

export function runningTerminals(
  terminals: readonly TerminalRecord[],
): TerminalRecord[] {
  return terminals
    .filter((terminal) => isTerminalRunning(terminal.phase))
    .toSorted((a, b) => Date.parse(a.startedAt) - Date.parse(b.startedAt))
}

export function terminalStatus(phase: TerminalPhase): ConversationStatus {
  return phase === 'running' ? 'active' : 'done'
}

export function runningChipLabel(count: number): string {
  return count === 1 ? '1 Running' : `${count} Running`
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
