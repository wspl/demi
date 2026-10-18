import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import type { SideQuestion } from '../api/generated/web-api'
import type { Conversation, SideRecord } from '../state/types'

/** A side question the list named, with nothing of its transcript held yet. */
export function sideRecord(record: SideQuestion): SideRecord {
  return { ...unlistedSide(record.id), record }
}

/**
 * A side question the socket's frames name before the list does: its
 * frames are kept, and it shows once the list names it.
 */
export function unlistedSide(id: string): SideRecord {
  return {
    id,
    record: null,
    history: EMPTY_TRANSCRIPT,
    failures: {},
    sessionPhase: 'idle',
    pendingCalls: [],
    waitingSince: null,
    sending: false,
    folding: null,
  }
}

/**
 * The side question `id` of `conversation`, as the socket's frames name it:
 * one the list has not named yet is held from its first frame on.
 */
export function sideFor(conversation: Pick<Conversation, 'sides'>, id: string): SideRecord {
  const held = conversation.sides.find((side) => side.id === id)
  if (held) {
    return held
  }
  conversation.sides.push(unlistedSide(id))
  return conversation.sides.at(-1)!
}
