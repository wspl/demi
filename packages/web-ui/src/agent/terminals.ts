import type { ConversationStatus } from './conversation-status'

export type TerminalPhase = 'running' | 'exited'

export interface TerminalRecord {
  id: string
  name: string
  phase: TerminalPhase
  startedAt: string
  endedAt?: string
  output: string
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
