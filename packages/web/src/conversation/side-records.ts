import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import type { AgentActivity } from '@demicodes/web-ui/agent/agent-activity'
import type { SideRecord } from '../state/types'

/** A side question, with nothing of its transcript held yet. */
export function sideRecord(record: SideRecord['record']): SideRecord {
  return {
    id: record.id,
    record,
    listed: false,
    history: EMPTY_TRANSCRIPT,
    shownAt: null,
    failures: {},
    sessionPhase: 'idle',
    pendingCalls: [],
    waitingSince: null,
    sent: null,
    folding: null,
  }
}

/** What the side question's agent does now, as its record holds it. */
export function sideActivity(side: SideRecord): AgentActivity {
  return {
    pendingCalls: side.pendingCalls,
    waitingSince: side.waitingSince,
    actionSent: side.sent !== null,
  }
}

/**
 * The page's action on its way stays so until what the socket brings shows
 * the server has it, not the backend's answer to the request, which may come
 * first: the agent at work, or, once the backend took it, a transcript
 * changed since the action was sent, as after a Retry whose whole turn ran
 * while the socket was away. So the window says Requesting, without a time,
 * from the press until the server tells its wait, without a gap, and nothing
 * once the turn ended unseen.
 */
export function settleSent(side: SideRecord): void {
  const sent = side.sent
  if (sent === null) {
    return
  }
  if (side.sessionPhase !== 'idle' || side.waitingSince !== null || (sent.taken && side.history !== sent.heldWhenSent)) {
    side.sent = null
  }
}
