import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
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
    sending: false,
    folding: null,
  }
}
