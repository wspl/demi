import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import { latestBlocks } from '@demicodes/web-ui/agent/history'
import type { AgentActivity } from '@demicodes/web-ui/agent/agent-activity'
import { waitClock } from '@demicodes/web-ui/agent/wait-clock'
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
    waitAction: null,
    sent: null,
    waitClock: waitClock(),
    folding: null,
  }
}

/** What the side question's agent does now, as its record holds it. */
export function sideActivity(side: SideRecord): AgentActivity {
  return {
    pendingCalls: side.pendingCalls,
    waitingSince: side.waitingSince,
    waitAction: side.waitAction,
    waitClock: side.waitClock,
    sentAction: side.sent?.id ?? null,
  }
}

/**
 * The page's action on its way stays so until what the socket brings shows
 * the server has it, not the backend's answer to the request, which may come
 * first: a wait the action began, its message in the transcript, or, once
 * the backend took it, a transcript changed since while the agent rests, as
 * after a Retry whose whole turn ran while the socket was away. So the window
 * says Requesting from the press, through the answer and the confirmation,
 * without a gap, and nothing once the turn ended unseen.
 */
export function settleSent(side: SideRecord): void {
  const sent = side.sent
  if (sent === null) {
    return
  }
  const shown = side.waitAction === sent.id
    || latestBlocks(side.history).some((block) => block.type === 'user' && block.turnId === sent.id)
    || (sent.heldWhenTaken !== null && side.history !== sent.heldWhenTaken && side.sessionPhase === 'idle' && side.waitingSince === null)
  if (shown) {
    side.sent = null
  }
}
