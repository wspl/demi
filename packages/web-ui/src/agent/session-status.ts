import type { SentenceText } from '../ui/ui-text'
import { isCompactionDivider } from './visible-blocks'

/** Sidebar conversation list, or a session that is not reconnecting. */
export type ListLoad = 'ready' | 'loading' | 'failed'

/** Loading covers an uncached opening and its handshake; cached navigation preserves the phase. */
export type SessionLoad = ListLoad | 'reconnecting'

/**
 * What the session pane shows instead of the transcript. `empty` is an open
 * conversation with no messages yet: the composer under it is the way in.
 * `none` is the chat route with no conversation open at all, so the pane
 * itself offers to start one.
 */
export type SessionStatusKind = 'loading' | 'failed' | 'empty' | 'missing' | 'none'

/**
 * The pane that replaces the transcript. `loading` always wins so a restore
 * never reads as an empty conversation, even with cached blocks. Reconnecting keeps the transcript
 * (or an empty list) and uses the same tail row as Requesting. A failure with
 * history in memory also keeps the transcript: the dock's notice names the
 * failure and offers Retry, and nothing the reader already had disappears.
 */
export function sessionPaneStatus(
  load: SessionLoad,
  hasTranscript: boolean,
): SessionStatusKind | null {
  if (load === 'loading') {
    return 'loading'
  }
  if (load === 'failed') {
    return hasTranscript ? null : 'failed'
  }
  if (load === 'reconnecting') {
    return null
  }
  return hasTranscript ? null : 'empty'
}

/** A session-level failure told at the tail of the transcript. */
export interface SessionFailureNotice {
  label: SentenceText
  /** Retry reopens the session; a refused request has nothing to reopen. */
  retry: boolean
}

/**
 * What the transcript's tail notice says for a session-level failure, or null
 * when another surface already says it: the status pane (no transcript to
 * keep) or the error record at the tail (it already says it).
 */
export function sessionFailureNotice(
  load: SessionLoad,
  lastError: string | null,
  hasTranscript: boolean,
  tailIsErrorRecord: boolean,
): SessionFailureNotice | null {
  if (!lastError) {
    return null
  }
  if (load === 'failed') {
    return hasTranscript ? { label: lastError, retry: true } : null
  }
  if (load === 'loading' || tailIsErrorRecord) {
    return null
  }
  return { label: lastError, retry: false }
}

/** The chat route when the sidebar list is not ready, or the id is unknown. */
export function conversationPageKind(
  list: ListLoad,
  found: boolean,
): SessionStatusKind | 'session' {
  if (found) {
    return 'session'
  }
  if (list === 'loading') {
    return 'loading'
  }
  if (list === 'failed') {
    return 'failed'
  }
  return 'missing'
}

export function sessionStatusCopy(kind: SessionStatusKind): {
  label: SentenceText
  action?: 'retry' | 'create'
} {
  if (kind === 'loading') {
    return { label: 'Loading conversation…' }
  }
  if (kind === 'failed') {
    return {
      label: 'Couldn’t load this conversation.',
      action: 'retry',
    }
  }
  if (kind === 'missing') {
    return {
      label: 'Conversation not found.',
      action: 'create',
    }
  }
  if (kind === 'none') {
    return {
      label: 'No conversation open.',
      action: 'create',
    }
  }
  return { label: 'No messages yet.' }
}

/**
 * The recovery the dock offers for a turn that did not finish (`product.md`
 * § Recovering an unfinished turn), from the record that ended the last
 * turn among the visible `blocks`: an error resumes, the user's own Stop
 * continues, and anything else is finished or still running. An error that
 * ended no turn, such as a Compact whose summary request failed, and a
 * compaction's boundary and marker leave the turn before them as it was
 * (`failures-and-recovery.md` § The unfinished turn). Both are the
 * session's `resume`; the word tells the cause.
 */
export function turnRecovery(
  phase: 'idle' | string,
  blocks: readonly RecordCandidate[],
): 'resume' | 'continue' | null {
  if (phase !== 'idle') {
    return null
  }
  const end = turnRecord(blocks)
  if (end?.type === 'error') {
    return 'resume'
  }
  return end?.type === 'abort' ? 'continue' : null
}

/** What `turnRecovery` reads of a block. */
interface RecordCandidate {
  type: string
  outsideTurn?: boolean
  code?: string | null
  device?: { id: string; name: string }
}

/** The record that ended the last turn among `blocks`, or the block that ended it otherwise. */
function turnRecord<T extends RecordCandidate>(blocks: readonly T[]): T | undefined {
  return blocks.findLast(
    (block) => !isCompactionDivider(block) && !(block.type === 'error' && block.outsideTurn),
  )
}

/**
 * The device whose return the dock's Resume waits for (`product.md`
 * § Recovering an unfinished turn): the turn ended unfinished because that
 * device was offline, and `online` says it still is not. Null when Resume
 * may go.
 */
export function resumeWaitsFor(
  blocks: readonly RecordCandidate[],
  online: (deviceId: string) => boolean,
): string | null {
  const end = turnRecord(blocks)
  if (end?.type !== 'error' || end.code !== 'host_offline' || !end.device) {
    return null
  }
  return online(end.device.id) ? null : end.device.name
}
