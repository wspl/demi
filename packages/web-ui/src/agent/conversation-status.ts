import type { SessionPhase } from '@demicodes/protocol'
import type { StatusDotTone } from '../ui/StatusDot.vue'
import type { ConversationState } from './types'
import { isSubagentRunning, type SubagentRecord } from './subagents'
import { isTerminalRunning, type TerminalRecord } from './terminals'

export type ConversationStatus = 'idle' | 'active' | 'done' | 'error' | 'aborted'

/**
 * The tone of a conversation's status dot wherever it shows, a sidebar row's, a
 * tab's or an agent's mark, so one state has one look everywhere:
 *
 * - running: the accent, which breathes
 * - finished: green
 * - failed: red
 *
 * Green and red say there is something new to see. Where the place knows
 * whether it was read, as a sidebar row, a read answer or failure settles. An
 * agent's tab and link have no read state, pass no `outcome`, and stay green or
 * red.
 *
 * A Stop is the user's own decision, neither finished nor failed. Like a
 * conversation that has run nothing yet, it has no tone: the place's settled
 * look.
 */
export function statusTone(status: ConversationStatus, outcome?: { read: boolean }): StatusDotTone | null {
  switch (status) {
    case 'active':
      return 'accent'
    case 'done':
      return outcome?.read ? null : 'success'
    case 'error':
      return outcome?.read ? null : 'danger'
    case 'aborted':
    case 'idle':
      return null
  }
}

/** Whether the session compacts, as the user asked or by itself while idle. */
export function isCompacting(phase: SessionPhase): boolean {
  return phase === 'compacting' || phase === 'compacting_idle'
}

/**
 * A conversation remains active until its root and all children finish, and
 * while a command that is not a service runs, whose end wakes its agent. A
 * service runs until stopped, so it never keeps the conversation active, nor
 * does the root's compaction while idle, which nobody asked for; the backend
 * counts the same way (`crates/agent-server/src/server/tree.rs`).
 */
export function isConversationActive(
  phase: ConversationState['phase'],
  subagents: readonly Pick<SubagentRecord, 'phase'>[] = [],
  terminals: readonly Pick<TerminalRecord, 'phase' | 'service'>[] = [],
): boolean {
  return (
    (phase !== 'idle' && phase !== 'compacting_idle') ||
    subagents.some((agent) => isSubagentRunning(agent.phase)) ||
    terminals.some((terminal) => isTerminalRunning(terminal.phase) && !terminal.service)
  )
}
