import { t } from '../infra/i18n'

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
  label: string
  /** Retry reopens the session; a refused request has nothing to reopen. */
  retry: boolean
}

/**
 * What the transcript's tail notice says for a session-level failure, or null
 * when another surface already says it: the status pane (no transcript to
 * keep) or the error record at the tail (it carries its own Retry).
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
  label: string
  action?: 'retry' | 'create'
} {
  if (kind === 'loading') {
    return { label: t('agent.session.loading') }
  }
  if (kind === 'failed') {
    return {
      label: t('agent.session.failed'),
      action: 'retry',
    }
  }
  if (kind === 'missing') {
    return {
      label: t('agent.session.missing'),
      action: 'create',
    }
  }
  if (kind === 'none') {
    return {
      label: t('agent.session.none'),
      action: 'create',
    }
  }
  return { label: t('agent.session.empty') }
}
