import type { PendingCall, ProviderFailureFacts, SessionPhase } from '@demicodes/protocol'
import type { HeldTranscript } from '@demicodes/conversation-client'
import type { ConversationStatus } from './conversation-status'
import type { MessageEditContent } from './message-editing'

export type SubagentPhase = 'running' | 'completed' | 'aborted' | 'error'

export interface SubagentRecord {
  id: string
  /** The number the model knows it by, which a message links it with as `agent:<number>`. */
  number: number
  name: string
  phase: SubagentPhase
  /**
   * What its session does while it is live, as its `phase` frames say: a
   * child idle beside a command it left running shows no Requesting row
   * (`runtime.md` § Frame protocol).
   */
  sessionPhase: SessionPhase
  startedAt: string
  endedAt?: string
  /** The parts of this agent's transcript the page holds, from its pages and its stream. */
  history: HeldTranscript
  /** The calls this agent's model is writing (`runtime.md` § Calls being written). */
  pendingCalls: PendingCall[]
  /**
   * When this agent began to wait for the model's answer, as `Date.now()`;
   * null when it waits for none (`runtime.md` § Waiting for the model).
   */
  waitingSince: number | null
  /** The failure facts of this agent's error blocks, by block id. */
  failures: Record<string, ProviderFailureFacts>
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

/**
 * A child's task brief, the `user` block its spawn or resume writes in the
 * child's transcript `node`: Markdown an agent wrote, which renders as an
 * answer does (`subagents.md` § Product rendering). Null for a `user` block
 * of the conversation's own transcript, which the user typed and which keeps
 * the user dialect, and for one that holds a file, which only a message the
 * user sent does.
 */
export function childBrief(node: string | null, content: readonly MessageEditContent[]): string | null {
  if (node === null) {
    return null
  }
  let brief = ''
  for (const part of content) {
    if (part.type !== 'text') {
      return null
    }
    brief += part.text
  }
  return brief
}

/**
 * The dock's agents chip (`subagents.md` § Product rendering): the live
 * children's count, and no chip while none runs, as the Running chip shows
 * only while a command runs. Finished children never count; they stay one
 * click away from their receipts and links.
 */
export function agentsChipLabel(agents: readonly SubagentRecord[]): string | null {
  const running = agents.filter((agent) => isSubagentRunning(agent.phase)).length
  if (running === 0) {
    return null
  }
  return running === 1 ? '1 Agent' : `${running} Agents`
}

/**
 * The panel's tabs (`subagents.md` § Product rendering): the running
 * children, and after them the finished one picked from Completed.
 */
export function subagentPanelTabs(
  agents: readonly SubagentRecord[],
  activeId: string | null,
): SubagentRecord[] {
  const running = runningSubagents(agents)
  const active = agents.find((agent) => agent.id === activeId)
  return active && !isSubagentRunning(active.phase) ? [...running, active] : running
}
