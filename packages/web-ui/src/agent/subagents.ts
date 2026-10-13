import type { ContextUsage, JobModel, PendingCall, ProviderFailureFacts, SessionPhase, SubagentJob } from '@demicodes/protocol'
import type { HeldTranscript } from '@demicodes/conversation-client'
import type { ConversationStatus } from './conversation-status'
import type { MessageEditContent } from './message-editing'
import { effortWord } from './reasoning'
import { formatSpan, countOf } from '@demicodes/utils'
import { elapsedBetween } from '../composables/useElapsedTime'
import { COMPACT_AFTER_TURN } from './context-usage'

export type SubagentPhase = 'running' | 'completed' | 'aborted' | 'error'

/**
 * What a child's round runs on and how far it got, as its job says
 * (`subagents.md` § What an agent runs on).
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
}

export interface SubagentRecord extends AgentRun {
  id: string
  /** The number the model knows it by, which a message links it with as `agent:<number>`. */
  number: number
  name: string
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
  /**
   * How full this agent's next request is, as its session last said
   * (`compaction.md` § Context estimate); null until it says, and for a
   * child that ended before the page opened.
   */
  contextUsage: ContextUsage | null
}

/**
 * Where a child's context card stands on Compact: whether the child
 * compacts now, and why Compact cannot run apart from the usage. A closed
 * child has no session to compact; a running turn compacts after it ends,
 * while a child idle beside a command it left running may compact.
 */
export function agentCompaction(agent: SubagentRecord): { compacting: boolean, unavailableReason: string | null } {
  if (agent.phase !== 'running') {
    return { compacting: false, unavailableReason: 'The agent has ended.' }
  }
  return {
    compacting: agent.sessionPhase === 'compacting',
    unavailableReason: agent.sessionPhase === 'running' ? COMPACT_AFTER_TURN : null,
  }
}

/** What a child's job sets on its record: who it is, and its round as the job says it now. */
export type SubagentJobFields = Pick<SubagentRecord, 'id' | 'number' | 'name'> & AgentRun

/**
 * A child's record fields from its job, a `subagent` frame's or the
 * subagents route's: a `started` job of a later round replaces the closed
 * round's start, end, phase and model.
 */
export function subagentJobFields(job: SubagentJob): SubagentJobFields {
  return {
    id: job.subagentId,
    number: job.number,
    name: job.description,
    profile: job.profile,
    model: job.model,
    round: job.round,
    forkedFrom: job.forkedFrom,
    phase: job.phase,
    startedAt: job.startedAt,
    endedAt: job.endedAt ?? undefined,
  }
}

/**
 * The effort a child's round runs with, in the words the model menu shows:
 * *Medium*, *Thinking off*; null for a model without a thinking setting
 * (`subagents.md` § What an agent runs on).
 */
export function runEffortWords(model: JobModel): string | null {
  if (model.effort === null) {
    return null
  }
  return model.effort === 'off' ? 'Thinking off' : effortWord(model.effort)
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

/** How long a child has run, or ran, in the duration words: a running one counts to `nowMs`. */
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
  return countOf(running, 'Agent')
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
