import type { AgentCost, AgentEntry, ContextUsage, JobModel, JobPhase, PendingCall, ProviderFailureFacts, SessionPhase, Subagent } from '@demicodes/protocol'
import type { AgentActivity } from './agent-activity'
import type { HeldTranscript } from '@demicodes/conversation-client'
import type { ConversationStatus } from './conversation-status'
import type { MessageEditContent } from './message-editing'
import { effortWord } from './reasoning'
import { formatSpan } from '../ui/dates'
import { t } from './locales/text'
import { elapsedBetween } from '../composables/useElapsedTime'
import { compactAfterTurn } from './context-usage'

export type SubagentPhase = JobPhase

/**
 * What a child's round runs on and how far it got, as its entry says.
 */
export interface AgentRun {
  /** The profile's name; null for the inherit profile. */
  profile: string | null
  model: JobModel
  /** 1 for the spawn's round, one more at each resume. */
  round: number
  /** The number of the agent it was forked from; null for one not forked. */
  forkedFrom: number | null
  phase: SubagentPhase
  startedAt: string
  endedAt?: string
  /**
   * Where its `shell` calls start now, on the primary Host
   * (`crates/agent-server/src/node/mod.rs`).
   */
  cwd: string
}

export interface SubagentRecord extends AgentRun {
  id: string
  /**
   * The number the model knows it by, which a message links it with as
   * `agent:<number>`.
   */
  number: number
  /** The node that spawned it. */
  parent: string
  name: string
  /**
   * Whether the call that started it is on its parent's current path: the
   * Agents window and `agent:` links show only the children that are.
   */
  onPath: boolean
  /**
   * What its session does while it is live, as its entry says: a child idle
   * beside a command it left running shows no Requesting row.
   */
  sessionPhase: SessionPhase
  startedAt: string
  endedAt?: string
  /**
   * The parts of this agent's transcript the page holds, from its pages and its
   * stream.
   */
  history: HeldTranscript
  /**
   * The block its view shows, which bounds what of its history the page
   * holds; null for the latest
   * (`docs/decisions/0025-a-transcript-is-shown-from-a-bounded-window.md`).
   */
  shownAt: string | null
  /** How many blocks of the window lie within the view's reach on either side of that block, which the window holds beyond (`PREFETCH_SCREENS`); absent for none. */
  shownWithin?: number
  /** The calls this agent's model is writing. */
  pendingCalls: PendingCall[]
  /**
   * When this agent began to wait for the model's answer, as `Date.now()`;
   * null when it waits for none.
   */
  waitingSince: number | null
  /** The failure facts of this agent's error blocks, by block id. */
  failures: Record<string, ProviderFailureFacts>
  /**
   * How full this agent's next request is, as its entry says; null for one
   * that is not live.
   */
  contextUsage: ContextUsage | null
  /** What its own calls cost, kept by the backend as they are recorded. */
  cost: AgentCost
}

/**
 * Where a running child's context card stands on Compact: whether the child
 * compacts now, and why Compact cannot run apart from the usage. A running
 * turn compacts after it ends, while a child idle beside a command it left
 * running may compact. An ended child has no context card at all;
 * the backend refuses to compact
 * its node.
 */
export function agentCompaction(agent: SubagentRecord): { compacting: boolean, unavailableReason: string | null } {
  return {
    compacting: agent.sessionPhase === 'compacting',
    unavailableReason: agent.sessionPhase === 'running' ? compactAfterTurn() : null,
  }
}

/**
 * What the child does now, as its record holds it: the page acts on no child
 * of its own, so no wait of it counts from a press of the page's.
 */
export function subagentActivity(agent: Pick<SubagentRecord, 'pendingCalls' | 'waitingSince'>): AgentActivity {
  return { pendingCalls: agent.pendingCalls, waitingSince: agent.waitingSince, actionSent: false }
}

/** What a child's entry sets on its record: all of it but what the page holds of its transcript. */
export type SubagentEntryFields = Omit<SubagentRecord, 'history' | 'shownAt' | 'failures'>

/**
 * A child's record fields from its entry `entry`, whose role is `child`, as
 * it arrived at `receivedAt`, as `Date.now()`: a later round's entry replaces
 * the closed round's start, end, phase and model.
 */
export function subagentEntryFields(entry: AgentEntry, child: Subagent, receivedAt: number): SubagentEntryFields {
  return {
    id: entry.id,
    number: child.number,
    parent: child.parent,
    name: child.description,
    onPath: child.onPath,
    profile: child.profile,
    model: child.model,
    round: child.round,
    forkedFrom: child.forkedFrom,
    phase: child.state,
    startedAt: child.startedAt,
    endedAt: child.endedAt ?? undefined,
    cwd: child.cwd,
    sessionPhase: entry.phase,
    pendingCalls: entry.pendingCalls,
    waitingSince: entry.waitedMs === null ? null : receivedAt - entry.waitedMs,
    contextUsage: entry.context,
    cost: entry.cost,
  }
}

/**
 * The effort a child's round runs with, in the words the model menu shows:
 * *Medium*, *Thinking off*; null for a model without a thinking setting.
 */
export function runEffortWords(model: JobModel): string | null {
  if (model.effort === null) {
    return null
  }
  return model.effort === 'off' ? t('subagents.thinkingOff') : effortWord(model.effort)
}

/**
 * What an agent is called: its description, or *Agent 4* for one spawned
 * without a description.
 */
export function agentName(description: string, number: number): string {
  return description.trim() || t('transcript.receipt.unnamed', { number }).trim()
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

/**
 * How long a child has run, or ran, in the duration words: a running one counts
 * to `nowMs`.
 */
export function formatSubagentDuration(
  agent: SubagentRecord,
  nowMs: number,
): string {
  const end = agent.endedAt ? Date.parse(agent.endedAt) : nowMs
  return formatSpan(elapsedBetween(Date.parse(agent.startedAt), end) ?? 0)
}

/**
 * A child's task brief, the `user` block its spawn or resume writes in the
 * child's transcript `node`: Markdown an agent wrote, which renders as an
 * answer does. Null for a `user` block
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
 * The dock's agents chip: the live
 * children's count, and no chip while none runs, as the Running chip shows
 * only while a command runs. Finished children never count; they stay one
 * click away from their receipts and links.
 */
export function agentsChipLabel(agents: readonly SubagentRecord[]): string | null {
  const running = agents.filter((agent) => isSubagentRunning(agent.phase)).length
  if (running === 0) {
    return null
  }
  return t('subagents.chip', { count: running }, running)
}

/**
 * The panel's tabs: the running
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
