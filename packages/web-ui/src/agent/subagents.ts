import type { Block } from '@demicodes/core'
import type { ConversationStatus } from './conversation-status'

export type SubagentPhase = 'running' | 'completed' | 'aborted' | 'error'

export interface SubagentRecord {
  id: string
  name: string
  phase: SubagentPhase
  startedAt: string
  endedAt?: string
  blocks: Block[]
}

export function isSubagentRunning(phase: SubagentPhase): boolean {
  return phase === 'running'
}

export function runningSubagents(
  agents: readonly SubagentRecord[],
): SubagentRecord[] {
  return agents
    .filter((agent) => isSubagentRunning(agent.phase))
    .toSorted((a, b) => Date.parse(a.startedAt) - Date.parse(b.startedAt))
}

export function finishedSubagents(
  agents: readonly SubagentRecord[],
): SubagentRecord[] {
  return agents
    .filter((agent) => !isSubagentRunning(agent.phase))
    .toSorted(
      (a, b) =>
        Date.parse(b.endedAt ?? b.startedAt) - Date.parse(a.endedAt ?? a.startedAt),
    )
}

export function firstRunningSubagentId(
  agents: readonly SubagentRecord[],
): string | null {
  return runningSubagents(agents)[0]?.id ?? null
}

export function firstFinishedSubagentId(
  agents: readonly SubagentRecord[],
): string | null {
  return finishedSubagents(agents)[0]?.id ?? null
}

/** Open running children first; if none are live, open the newest finished child. */
export function firstInspectSubagentId(
  agents: readonly SubagentRecord[],
): string | null {
  return firstRunningSubagentId(agents) ?? firstFinishedSubagentId(agents)
}

export function subagentStatus(phase: SubagentPhase): ConversationStatus {
  if (phase === 'running') {
    return 'active'
  }
  if (phase === 'error') {
    return 'error'
  }
  if (phase === 'aborted') {
    return 'aborted'
  }
  return 'done'
}

export function subagentIndicator(
  phase: SubagentPhase,
): 'accent' | 'success' | 'danger' | 'muted' {
  if (phase === 'running') {
    return 'accent'
  }
  if (phase === 'completed') {
    return 'success'
  }
  if (phase === 'error' || phase === 'aborted') {
    return 'danger'
  }
  return 'muted'
}

/** Compact elapsed time for roster rows: `12s`, `2m`, `2m14s`, `1h3m`. */
export function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000))
  if (totalSeconds < 60) {
    return `${totalSeconds}s`
  }
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  if (minutes < 60) {
    return seconds > 0 ? `${minutes}m${seconds}s` : `${minutes}m`
  }
  const hours = Math.floor(minutes / 60)
  const remMinutes = minutes % 60
  return remMinutes > 0 ? `${hours}h${remMinutes}m` : `${hours}h`
}

export function subagentDurationMs(agent: SubagentRecord, nowMs: number): number {
  const start = Date.parse(agent.startedAt)
  const end = agent.endedAt ? Date.parse(agent.endedAt) : nowMs
  return Math.max(0, end - start)
}

export function formatSubagentDuration(
  agent: SubagentRecord,
  nowMs: number,
): string {
  return formatDuration(subagentDurationMs(agent, nowMs))
}

export function agentsChipLabel(count: number): string {
  return count === 1 ? '1 Agent' : `${count} Agents`
}

/** Running tabs stay up; a finished inspect adds that child beside them. */
export function subagentPanelTabs(
  agents: readonly SubagentRecord[],
  activeId: string | null,
): SubagentRecord[] {
  if (!activeId) {
    return []
  }
  const active = agents.find((agent) => agent.id === activeId)
  if (!active) {
    return []
  }
  const running = runningSubagents(agents)
  if (isSubagentRunning(active.phase)) {
    return running
  }
  return [...running, active]
}
