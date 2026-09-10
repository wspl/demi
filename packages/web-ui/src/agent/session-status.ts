import { t } from '../infra/i18n'

/** Sidebar conversation list, or a session that is not reconnecting. */
export type ListLoad = 'ready' | 'loading' | 'failed'

/** Loading includes navigation and its initial handshake; reconnecting is a dropped open connection. */
export type SessionLoad = ListLoad | 'reconnecting'

/** What the session pane shows instead of the transcript. */
export type SessionStatusKind = 'loading' | 'failed' | 'empty' | 'missing'

/**
 * The pane that replaces the transcript. `loading` always wins so a restore
 * never reads as an empty conversation, even with cached blocks. Reconnecting keeps the transcript
 * (or an empty list) and uses the same tail row as Requesting.
 */
export function sessionPaneStatus(
  load: SessionLoad,
  hasTranscript: boolean,
): SessionStatusKind | null {
  if (load === 'loading') {
    return 'loading'
  }
  if (load === 'failed') {
    return 'failed'
  }
  if (load === 'reconnecting') {
    return null
  }
  return hasTranscript ? null : 'empty'
}

export function sessionShowsReconnectTail(load: SessionLoad): boolean {
  return load === 'reconnecting'
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
  return { label: t('agent.session.empty') }
}
